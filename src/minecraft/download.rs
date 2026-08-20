use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::Client;
use sha1::{Digest, Sha1};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

use super::metadata::Download;

pub async fn fetch_bytes(
    client: &Client,
    url: &str,
    expected_sha1: Option<&str>,
) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to request {url}"))?
        .error_for_status()
        .with_context(|| format!("server rejected {url}"))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read {url}"))?
        .to_vec();
    if let Some(expected) = expected_sha1.filter(|value| !value.is_empty()) {
        verify_bytes(&bytes, expected, url)?;
    }
    Ok(bytes)
}

pub async fn download_to(client: &Client, download: &Download, target: &Path) -> Result<()> {
    if file_is_valid(target, download).await? {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let temporary = temporary_path(target);
    let _ = fs::remove_file(&temporary).await;
    let response = client
        .get(&download.url)
        .send()
        .await
        .with_context(|| format!("failed to request {}", download.url))?
        .error_for_status()
        .with_context(|| format!("server rejected {}", download.url))?;
    let mut stream = response.bytes_stream();
    let mut file = fs::File::create(&temporary)
        .await
        .with_context(|| format!("failed to create {}", temporary.display()))?;
    let mut hasher = Sha1::new();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("failed to download {}", download.url))?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        size += chunk.len() as u64;
    }
    file.flush().await?;

    if download.size > 0 && size != download.size {
        let _ = fs::remove_file(&temporary).await;
        bail!(
            "size mismatch for {}: expected {}, received {}",
            download.url,
            download.size,
            size
        );
    }
    if !download.sha1.is_empty() {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&download.sha1) {
            let _ = fs::remove_file(&temporary).await;
            bail!(
                "checksum mismatch for {}: expected {}, received {}",
                download.url,
                download.sha1,
                actual
            );
        }
    }
    let _ = fs::remove_file(target).await;
    fs::rename(&temporary, target)
        .await
        .with_context(|| format!("failed to install {}", target.display()))?;
    Ok(())
}

async fn file_is_valid(path: &Path, download: &Download) -> Result<bool> {
    let metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if download.size > 0 && metadata.len() != download.size {
        return Ok(false);
    }
    if download.sha1.is_empty() {
        return Ok(metadata.len() > 0);
    }

    let mut file = fs::File::open(path).await?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(&download.sha1))
}

fn verify_bytes(bytes: &[u8], expected: &str, label: &str) -> Result<()> {
    let actual = format!("{:x}", Sha1::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("checksum mismatch for {label}: expected {expected}, received {actual}");
    }
    Ok(())
}

fn temporary_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    target.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_verification_rejects_invalid_content() {
        assert!(verify_bytes(b"hello", "not-a-checksum", "test").is_err());
        assert!(verify_bytes(b"hello", "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d", "test").is_ok());
    }
}
