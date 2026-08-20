use std::{
    collections::HashSet,
    io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use futures_util::{TryStreamExt, stream};
use reqwest::Client;
use tokio::{fs, sync::mpsc::UnboundedSender};

use super::{
    TaskEvent, VERSION_MANIFEST_URL,
    download::{download_to, fetch_bytes},
    metadata::{
        AssetIndex, Download, NativeLibrary, Platform, VersionManifest, VersionMetadata,
        VersionSummary, maven_download, native_library, rules_allow,
    },
    paths::MinecraftPaths,
};

#[derive(Clone)]
struct InstallFile {
    download: Download,
    path: PathBuf,
    label: String,
}

pub async fn fetch_manifest(client: &Client) -> Result<VersionManifest> {
    let bytes = fetch_bytes(client, VERSION_MANIFEST_URL, None).await?;
    serde_json::from_slice(&bytes).context("failed to parse the Minecraft version manifest")
}

pub async fn installed_versions(paths: &MinecraftPaths) -> Result<Vec<String>> {
    let mut versions = Vec::new();
    let mut entries = match fs::read_dir(paths.versions()).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(versions),
        Err(error) => return Err(error.into()),
    };
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        if paths.version_json(&id).is_file() && paths.version_jar(&id).is_file() {
            versions.push(id);
        }
    }
    versions.sort_by(|left, right| right.cmp(left));
    Ok(versions)
}

pub async fn delete_version(paths: &MinecraftPaths, id: &str) -> Result<()> {
    validate_version_id(id)?;
    let directory = paths.version_dir(id);
    fs::remove_dir_all(&directory)
        .await
        .with_context(|| format!("failed to delete {}", directory.display()))
}

fn validate_version_id(id: &str) -> Result<()> {
    let mut components = Path::new(id).components();
    let is_single_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if id.is_empty() || id.contains(['/', '\\']) || !is_single_component {
        bail!("invalid Minecraft version ID: {id}");
    }
    Ok(())
}

pub async fn load_metadata(paths: &MinecraftPaths, id: &str) -> Result<VersionMetadata> {
    let path = paths.version_json(id);
    let bytes = fs::read(&path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

pub async fn install_version(
    client: Client,
    paths: MinecraftPaths,
    version: VersionSummary,
    events: UnboundedSender<TaskEvent>,
) -> Result<()> {
    validate_version_id(&version.id)?;
    events
        .send(TaskEvent::Status(format!(
            "Reading metadata for {}",
            version.id
        )))
        .ok();
    let metadata_bytes = fetch_bytes(&client, &version.url, Some(&version.sha1)).await?;
    let metadata: VersionMetadata = serde_json::from_slice(&metadata_bytes)
        .with_context(|| format!("failed to parse metadata for {}", version.id))?;
    let version_dir = paths.version_dir(&version.id);
    fs::create_dir_all(&version_dir).await?;

    events
        .send(TaskEvent::Status(format!("Installing {}", version.id)))
        .ok();
    let platform = Platform::current();
    let mut libraries = Vec::new();
    let mut native_files = Vec::new();
    let mut native_archives = Vec::new();
    let mut seen = HashSet::new();
    for library in &metadata.libraries {
        if !rules_allow(&library.rules, &platform) {
            continue;
        }
        let native = native_library(library, &platform);
        let is_coordinate_native = matches!(native.as_ref(), Some((NativeLibrary::Coordinate, _)));
        if let Some((kind, classifier)) = native {
            let download = match kind {
                NativeLibrary::Mapped => library
                    .downloads
                    .classifiers
                    .get(&classifier)
                    .cloned()
                    .or_else(|| maven_download(library, Some(&classifier))),
                NativeLibrary::Coordinate => library
                    .downloads
                    .artifact
                    .clone()
                    .or_else(|| maven_download(library, Some(&classifier))),
            };
            if let Some(download) = download {
                let path = paths.libraries().join(&download.path);
                if seen.insert(path.clone()) {
                    native_files.push(InstallFile {
                        label: format!("{} ({classifier})", library.name),
                        download,
                        path: path.clone(),
                    });
                }
                native_archives.push((path, library.extract.clone()));
            }
        }
        if !is_coordinate_native
            && let Some(download) = library
                .downloads
                .artifact
                .clone()
                .or_else(|| maven_download(library, None))
        {
            let path = paths.libraries().join(&download.path);
            if seen.insert(path.clone()) {
                libraries.push(InstallFile {
                    label: library.name.clone(),
                    download,
                    path,
                });
            }
        }
    }
    download_files(&client, libraries, &events).await?;
    download_files(&client, native_files, &events).await?;

    if let Some(asset_download) = &metadata.asset_index {
        let index_id = if asset_download.id.is_empty() {
            metadata.assets.as_str()
        } else {
            asset_download.id.as_str()
        };
        let index_path = paths.asset_index(index_id);
        download_files(
            &client,
            vec![InstallFile {
                download: asset_download.clone(),
                path: index_path.clone(),
                label: "asset index".to_owned(),
            }],
            &events,
        )
        .await?;
        let index_bytes = fs::read(&index_path).await?;
        let index: AssetIndex = serde_json::from_slice(&index_bytes)
            .with_context(|| format!("failed to parse asset index {index_id}"))?;
        let assets = index
            .objects
            .iter()
            .map(|(name, object)| InstallFile {
                download: Download {
                    id: String::new(),
                    path: String::new(),
                    sha1: object.hash.clone(),
                    size: object.size,
                    url: format!(
                        "https://resources.download.minecraft.net/{}/{}",
                        &object.hash[..2],
                        object.hash
                    ),
                },
                path: paths.asset_object(&object.hash),
                label: name.clone(),
            })
            .collect();
        download_files(&client, assets, &events).await?;
        if index.is_virtual || index.map_to_resources {
            materialize_legacy_assets(&paths, index_id, &index).await?;
        }
    }

    if !native_archives.is_empty() {
        events
            .send(TaskEvent::Status("Extracting native libraries".to_owned()))
            .ok();
        let native_dir = paths.natives(&version.id);
        let _ = fs::remove_dir_all(&native_dir).await;
        fs::create_dir_all(&native_dir).await?;
        for (archive, extract) in native_archives {
            let excludes = extract.map_or_else(Vec::new, |value| value.exclude);
            extract_archive(archive, native_dir.clone(), excludes).await?;
        }
    }

    let mut primary = vec![InstallFile {
        download: metadata.downloads.client.clone(),
        path: paths.version_jar(&version.id),
        label: "client".to_owned(),
    }];
    if let Some(logging) = &metadata.logging {
        let name = download_name(&logging.client.file, "client-log.xml");
        primary.push(InstallFile {
            download: logging.client.file.clone(),
            path: paths.assets().join("log_configs").join(name),
            label: "logging configuration".to_owned(),
        });
    }
    download_files(&client, primary, &events).await?;

    write_atomic(&paths.version_json(&version.id), &metadata_bytes).await?;

    events.send(TaskEvent::Installed(version.id)).ok();
    Ok(())
}

async fn download_files(
    client: &Client,
    files: Vec<InstallFile>,
    events: &UnboundedSender<TaskEvent>,
) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let total = files.len();
    let completed = Arc::new(AtomicUsize::new(0));
    let client = client.clone();
    let events = events.clone();
    stream::iter(files.into_iter().map(Ok::<_, anyhow::Error>))
        .try_for_each_concurrent(16, |file| {
            let client = client.clone();
            let events = events.clone();
            let completed = Arc::clone(&completed);
            async move {
                download_to(&client, &file.download, &file.path).await?;
                let current = completed.fetch_add(1, Ordering::Relaxed) + 1;
                events
                    .send(TaskEvent::Progress {
                        completed: current,
                        total,
                        label: file.label,
                    })
                    .ok();
                Ok(())
            }
        })
        .await
}

async fn materialize_legacy_assets(
    paths: &MinecraftPaths,
    index_id: &str,
    index: &AssetIndex,
) -> Result<()> {
    for (name, object) in &index.objects {
        let relative = Path::new(name);
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            bail!("asset index contains an unsafe path: {name}");
        }
        let base = if index.map_to_resources {
            paths.game_dir().join("resources")
        } else {
            paths.assets().join("virtual").join(index_id)
        };
        let destination = base.join(relative);
        if destination.exists() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::copy(paths.asset_object(&object.hash), destination).await?;
    }
    Ok(())
}

async fn extract_archive(
    archive: PathBuf,
    destination: PathBuf,
    excludes: Vec<String>,
) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = std::fs::File::open(&archive)
            .with_context(|| format!("failed to open {}", archive.display()))?;
        let mut zip = zip::ZipArchive::new(file)
            .with_context(|| format!("failed to read {}", archive.display()))?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let Some(relative) = entry.enclosed_name() else {
                continue;
            };
            let normalized = relative.to_string_lossy().replace('\\', "/");
            if normalized.starts_with("META-INF/")
                || excludes.iter().any(|prefix| normalized.starts_with(prefix))
                || entry.is_dir()
            {
                continue;
            }
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut output = std::fs::File::create(&target)?;
            io::copy(&mut entry, &mut output)?;
        }
        Ok(())
    })
    .await??;
    Ok(())
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).await?;
    let _ = fs::remove_file(path).await;
    fs::rename(temporary, path).await?;
    Ok(())
}

pub fn download_name(download: &Download, fallback: &str) -> String {
    if !download.id.is_empty() {
        return download.id.clone();
    }
    download
        .url
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deletes_only_the_selected_version_directory() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let paths = MinecraftPaths::new(temporary.path().join("minecraft"));
        fs::create_dir_all(paths.version_dir("1.21.8"))
            .await
            .unwrap();
        fs::create_dir_all(paths.version_dir("1.21.7"))
            .await
            .unwrap();
        fs::create_dir_all(paths.libraries()).await.unwrap();

        delete_version(&paths, "1.21.8").await.unwrap();

        assert!(!paths.version_dir("1.21.8").exists());
        assert!(paths.version_dir("1.21.7").exists());
        assert!(paths.libraries().exists());
    }

    #[tokio::test]
    async fn rejects_unsafe_version_ids() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let paths = MinecraftPaths::new(temporary.path().join("minecraft"));
        for id in [
            "",
            ".",
            "..",
            "../outside",
            "nested/version",
            "nested\\version",
        ] {
            assert!(delete_version(&paths, id).await.is_err(), "accepted {id}");
        }
    }
}
