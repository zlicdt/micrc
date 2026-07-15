use std::{collections::HashMap, path::PathBuf, process::Stdio};

use anyhow::{Context, Result, bail};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::UnboundedSender,
};

use crate::config::{Account, AccountKind, Settings};

use super::{
    TaskEvent,
    install::download_name,
    metadata::{Platform, VersionMetadata, expand_arguments, maven_download, rules_allow},
    paths::MinecraftPaths,
};

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
}

pub fn build_launch_command(
    metadata: &VersionMetadata,
    account: &Account,
    settings: &Settings,
    paths: &MinecraftPaths,
) -> Result<LaunchCommand> {
    if settings.min_memory_mb == 0 || settings.max_memory_mb < settings.min_memory_mb {
        bail!("Maximum memory must be greater than or equal to minimum memory");
    }

    let platform = Platform::current();
    let separator = if cfg!(target_os = "windows") {
        ";"
    } else {
        ":"
    };
    let mut classpath = Vec::new();
    for library in &metadata.libraries {
        if !rules_allow(&library.rules, &platform) {
            continue;
        }
        if let Some(download) = library
            .downloads
            .artifact
            .clone()
            .or_else(|| maven_download(library, None))
        {
            classpath.push(paths.libraries().join(download.path));
        }
    }
    classpath.push(paths.version_jar(&metadata.id));
    let classpath = classpath
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator);

    let asset_index_name = metadata
        .asset_index
        .as_ref()
        .map(|download| {
            if download.id.is_empty() {
                metadata.assets.as_str()
            } else {
                download.id.as_str()
            }
        })
        .unwrap_or(&metadata.assets);
    let mut replacements = HashMap::new();
    replacements.insert("auth_player_name", account.username.clone());
    replacements.insert("version_name", metadata.id.clone());
    replacements.insert(
        "game_directory",
        paths.game_dir().to_string_lossy().into_owned(),
    );
    replacements.insert("assets_root", paths.assets().to_string_lossy().into_owned());
    replacements.insert("assets_index_name", asset_index_name.to_owned());
    replacements.insert("auth_uuid", account.id.clone());
    let access_token = if account.kind == AccountKind::Microsoft {
        account.access_token.clone()
    } else {
        "0".to_owned()
    };
    replacements.insert("auth_access_token", access_token.clone());
    replacements.insert("auth_session", access_token);
    replacements.insert("clientid", String::new());
    replacements.insert("auth_xuid", String::new());
    replacements.insert(
        "user_type",
        if account.kind == AccountKind::Microsoft {
            "msa"
        } else {
            "legacy"
        }
        .to_owned(),
    );
    replacements.insert("version_type", metadata.kind.clone());
    replacements.insert("user_properties", "{}".to_owned());
    replacements.insert("profile_properties", "{}".to_owned());
    replacements.insert("resolution_width", settings.width.to_string());
    replacements.insert("resolution_height", settings.height.to_string());
    replacements.insert(
        "natives_directory",
        paths.natives(&metadata.id).to_string_lossy().into_owned(),
    );
    replacements.insert("launcher_name", env!("CARGO_PKG_NAME").to_owned());
    replacements.insert("launcher_version", env!("CARGO_PKG_VERSION").to_owned());
    replacements.insert(
        "library_directory",
        paths.libraries().to_string_lossy().into_owned(),
    );
    replacements.insert("classpath_separator", separator.to_owned());
    replacements.insert("classpath", classpath.clone());

    let mut jvm = vec![
        format!("-Xms{}M", settings.min_memory_mb),
        format!("-Xmx{}M", settings.max_memory_mb),
    ];
    jvm.extend(settings.extra_jvm_args.iter().cloned());
    if let Some(arguments) = &metadata.arguments {
        jvm.extend(expand_arguments(&arguments.jvm, &platform));
    } else {
        jvm.push(format!(
            "-Djava.library.path={}",
            paths.natives(&metadata.id).display()
        ));
        jvm.push("-cp".to_owned());
        jvm.push(classpath);
    }
    if let Some(logging) = &metadata.logging {
        let name = download_name(&logging.client.file, "client-log.xml");
        let path = paths.assets().join("log_configs").join(name);
        jvm.push(
            logging
                .client
                .argument
                .replace("${path}", &path.to_string_lossy()),
        );
    }
    let mut game = if let Some(arguments) = &metadata.arguments {
        expand_arguments(&arguments.game, &platform)
    } else {
        split_legacy_arguments(metadata.minecraft_arguments.as_deref().unwrap_or_default())?
    };
    jvm = jvm
        .into_iter()
        .map(|value| replace_placeholders(&value, &replacements))
        .collect();
    game = game
        .into_iter()
        .map(|value| replace_placeholders(&value, &replacements))
        .collect();

    let mut args = jvm;
    args.push(metadata.main_class.clone());
    args.extend(game);
    Ok(LaunchCommand {
        program: settings.java_path.clone(),
        args,
        working_directory: paths.game_dir(),
    })
}

pub async fn run_game(
    command: LaunchCommand,
    required_java: Option<u32>,
    events: UnboundedSender<TaskEvent>,
) -> Result<()> {
    if let Some(required) = required_java {
        let installed = detect_java_major(&command.program).await?;
        if installed < required {
            bail!(
                "Minecraft requires Java {required}, but {} is Java {installed}",
                command.program
            );
        }
    }
    tokio::fs::create_dir_all(&command.working_directory).await?;
    events
        .send(TaskEvent::Status(format!(
            "Starting Minecraft with {}",
            command.program
        )))
        .ok();
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false)
        .spawn()
        .with_context(|| format!("failed to start {}", command.program))?;

    let stdout = child
        .stdout
        .take()
        .context("failed to capture game output")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture game errors")?;
    let stdout_task = forward_lines(stdout, events.clone());
    let stderr_task = forward_lines(stderr, events.clone());
    let (status, _, _) = tokio::join!(child.wait(), stdout_task, stderr_task);
    let status = status.context("failed to wait for Minecraft")?;
    events.send(TaskEvent::GameExited(status.code())).ok();
    Ok(())
}

async fn forward_lines<R>(reader: R, events: UnboundedSender<TaskEvent>) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        events.send(TaskEvent::Log(line)).ok();
    }
    Ok(())
}

async fn detect_java_major(program: &str) -> Result<u32> {
    let output = Command::new(program)
        .arg("-version")
        .output()
        .await
        .with_context(|| format!("failed to execute {program} -version"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_java_major(&text).context("failed to understand the installed Java version")
}

fn parse_java_major(text: &str) -> Option<u32> {
    let marker = text.find("version")?;
    let version = text[marker + "version".len()..]
        .trim_start()
        .trim_start_matches('"');
    let digits: String = version
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let mut parts = digits.split('.');
    let first = parts.next()?.parse::<u32>().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

fn replace_placeholders(value: &str, replacements: &HashMap<&str, String>) -> String {
    let mut result = value.to_owned();
    for (key, replacement) in replacements {
        result = result.replace(&format!("${{{key}}}"), replacement);
    }
    result
}

fn split_legacy_arguments(value: &str) -> Result<Vec<String>> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if character == '\'' || character == '"' {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                arguments.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() {
        bail!("legacy game arguments contain an unterminated quote");
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AccountKind;

    #[test]
    fn parses_old_and_new_java_versions() {
        assert_eq!(parse_java_major("java version \"1.8.0_402\""), Some(8));
        assert_eq!(parse_java_major("openjdk version \"21.0.2\""), Some(21));
    }

    #[test]
    fn splits_quoted_legacy_arguments() {
        assert_eq!(
            split_legacy_arguments("--username Player --title \"Hello world\"").unwrap(),
            vec!["--username", "Player", "--title", "Hello world"]
        );
    }

    #[test]
    fn uses_microsoft_credentials_in_modern_arguments() {
        let metadata: VersionMetadata = serde_json::from_value(serde_json::json!({
            "id": "test-version",
            "type": "release",
            "mainClass": "com.example.Main",
            "assets": "test-assets",
            "downloads": {
                "client": { "url": "https://example.invalid/client.jar" }
            },
            "libraries": [],
            "arguments": {
                "jvm": ["-cp", "${classpath}"],
                "game": [
                    "--username", "${auth_player_name}",
                    "--accessToken", "${auth_access_token}",
                    "--userType", "${user_type}"
                ]
            }
        }))
        .unwrap();
        let account = Account {
            id: "0123456789abcdef0123456789abcdef".to_owned(),
            username: "Alex".to_owned(),
            kind: AccountKind::Microsoft,
            access_token: "minecraft-access-token".to_owned(),
            refresh_token: "refresh-token".to_owned(),
            expires_at: u64::MAX,
        };
        let temporary = temp_dir::TempDir::new().unwrap();
        let command = build_launch_command(
            &metadata,
            &account,
            &Settings::default(),
            &MinecraftPaths::new(temporary.path()),
        )
        .unwrap();
        assert!(command.args.iter().any(|value| value == "Alex"));
        assert!(
            command
                .args
                .iter()
                .any(|value| value == "minecraft-access-token")
        );
        assert!(command.args.iter().any(|value| value == "msa"));
    }
}
