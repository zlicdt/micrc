use std::{
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::config::{Account, AccountKind};

use super::TaskEvent;

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX_USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MINECRAFT_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const OAUTH_SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    message: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OAuthToken {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XboxUserRequest<'a> {
    properties: XboxUserProperties<'a>,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XboxUserProperties<'a> {
    auth_method: &'static str,
    site_name: &'static str,
    rps_ticket: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsRequest<'a> {
    properties: XstsProperties<'a>,
    relying_party: &'static str,
    token_type: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct XstsProperties<'a> {
    sandbox_id: &'static str,
    user_tokens: [&'a str; 1],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxTokenResponse {
    token: String,
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct DisplayClaims {
    xui: Vec<UserHash>,
}

#[derive(Debug, Deserialize)]
struct UserHash {
    uhs: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MinecraftLoginRequest {
    identity_token: String,
}

#[derive(Debug, Deserialize)]
struct MinecraftToken {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct Entitlements {
    items: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

#[derive(Debug, PartialEq, Eq)]
struct BrowserCommand {
    program: &'static str,
    arguments: Vec<String>,
}

pub async fn open_browser(url: &str) -> Result<()> {
    let launcher = browser_command(url)?;
    let mut command = Command::new(launcher.program);
    command
        .args(launcher.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", launcher.program))?;
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => bail!("{} exited with {status}", launcher.program),
        Ok(Err(error)) => Err(error).with_context(|| format!("{} failed", launcher.program)),
        Err(_) => {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(())
        }
    }
}

fn browser_command(url: &str) -> Result<BrowserCommand> {
    if !url.starts_with("https://") {
        bail!("refusing to open a non-HTTPS login URL");
    }

    #[cfg(target_os = "windows")]
    let command = BrowserCommand {
        program: "rundll32",
        arguments: vec!["url.dll,FileProtocolHandler".to_owned(), url.to_owned()],
    };
    #[cfg(target_os = "macos")]
    let command = BrowserCommand {
        program: "open",
        arguments: vec![url.to_owned()],
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = BrowserCommand {
        program: "xdg-open",
        arguments: vec![url.to_owned()],
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    bail!("opening a browser is not supported on this platform");

    Ok(command)
}

pub async fn login(
    client: Client,
    client_id: String,
    events: UnboundedSender<TaskEvent>,
    mut cancelled: oneshot::Receiver<()>,
) -> Result<Account> {
    let response = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id.as_str()), ("scope", OAUTH_SCOPE)])
        .send()
        .await
        .context("failed to start Microsoft device login")?;
    let device: DeviceCodeResponse = parse_response(response, "Microsoft device login").await?;
    let browser_url = device
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device.verification_uri);
    let browser_result = open_browser(browser_url).await;
    events
        .send(TaskEvent::DeviceCode {
            user_code: device.user_code.clone(),
            verification_uri: device.verification_uri.clone(),
            message: device.message.clone(),
            browser_opened: browser_result.is_ok(),
            browser_error: browser_result.err().map(|error| error.to_string()),
        })
        .ok();

    let started = tokio::time::Instant::now();
    let mut interval = device.interval.unwrap_or(5).max(1);
    let oauth = loop {
        if started.elapsed().as_secs() >= device.expires_in {
            bail!("Microsoft device login expired");
        }
        tokio::select! {
            _ = &mut cancelled => bail!("Microsoft login cancelled"),
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
        }
        let response = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device.device_code.as_str()),
            ])
            .send()
            .await
            .context("failed to poll Microsoft device login")?;
        if response.status().is_success() {
            break response.json::<OAuthToken>().await?;
        }
        let error: OAuthError = response
            .json()
            .await
            .context("Microsoft returned an invalid device login error")?;
        match error.error.as_str() {
            "authorization_pending" => continue,
            "slow_down" => {
                interval += 5;
                continue;
            }
            _ => bail!(
                "Microsoft login failed: {}",
                error.error_description.unwrap_or(error.error)
            ),
        }
    };

    exchange_for_minecraft(
        &client,
        &oauth.access_token,
        oauth.refresh_token.unwrap_or_default(),
    )
    .await
}

pub async fn refresh_if_needed(
    client: &Client,
    client_id: &str,
    account: &Account,
) -> Result<Account> {
    if account.kind != AccountKind::Microsoft || account.expires_at > unix_time() + 120 {
        return Ok(account.clone());
    }
    if client_id.is_empty() {
        bail!("Microsoft client ID is required to refresh this account");
    }
    if account.refresh_token.is_empty() {
        bail!("Microsoft session expired; sign in again");
    }
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", account.refresh_token.as_str()),
            ("scope", OAUTH_SCOPE),
        ])
        .send()
        .await
        .context("failed to refresh Microsoft login")?;
    let oauth: OAuthToken = parse_response(response, "Microsoft token refresh").await?;
    exchange_for_minecraft(
        client,
        &oauth.access_token,
        oauth
            .refresh_token
            .unwrap_or_else(|| account.refresh_token.clone()),
    )
    .await
}

async fn exchange_for_minecraft(
    client: &Client,
    microsoft_access_token: &str,
    refresh_token: String,
) -> Result<Account> {
    let rps_ticket = format!("d={microsoft_access_token}");
    let response = client
        .post(XBOX_USER_AUTH_URL)
        .json(&XboxUserRequest {
            properties: XboxUserProperties {
                auth_method: "RPS",
                site_name: "user.auth.xboxlive.com",
                rps_ticket: &rps_ticket,
            },
            relying_party: "http://auth.xboxlive.com",
            token_type: "JWT",
        })
        .send()
        .await
        .context("failed to authenticate with Xbox Live")?;
    let xbox: XboxTokenResponse = parse_response(response, "Xbox Live authentication").await?;
    let user_hash = xbox
        .display_claims
        .xui
        .first()
        .context("Xbox Live did not return a user hash")?
        .uhs
        .clone();

    let response = client
        .post(XSTS_URL)
        .json(&XstsRequest {
            properties: XstsProperties {
                sandbox_id: "RETAIL",
                user_tokens: [&xbox.token],
            },
            relying_party: "rp://api.minecraftservices.com/",
            token_type: "JWT",
        })
        .send()
        .await
        .context("failed to authorize Minecraft with XSTS")?;
    let xsts: XboxTokenResponse = parse_response(response, "Minecraft XSTS authorization").await?;

    let response = client
        .post(MINECRAFT_LOGIN_URL)
        .json(&MinecraftLoginRequest {
            identity_token: format!("XBL3.0 x={user_hash};{}", xsts.token),
        })
        .send()
        .await
        .context("failed to authenticate with Minecraft services")?;
    let minecraft: MinecraftToken = parse_response(response, "Minecraft authentication").await?;

    let response = client
        .get(ENTITLEMENTS_URL)
        .bearer_auth(&minecraft.access_token)
        .send()
        .await
        .context("failed to check Minecraft ownership")?;
    let entitlements: Entitlements = parse_response(response, "Minecraft ownership check").await?;
    let owns_java = entitlements.items.iter().any(|item| {
        item.get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name == "product_minecraft" || name == "game_minecraft")
    });
    if !owns_java {
        bail!("This Microsoft account does not own Minecraft: Java Edition");
    }

    let response = client
        .get(PROFILE_URL)
        .bearer_auth(&minecraft.access_token)
        .send()
        .await
        .context("failed to load the Minecraft profile")?;
    let profile: MinecraftProfile = parse_response(response, "Minecraft profile").await?;
    Ok(Account {
        id: profile.id,
        username: profile.name,
        kind: AccountKind::Microsoft,
        access_token: minecraft.access_token,
        refresh_token,
        expires_at: unix_time() + minecraft.expires_in.saturating_sub(60),
    })
}

async fn parse_response<T>(response: Response, operation: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .with_context(|| format!("{operation} returned invalid data"));
    }
    let message = response.text().await.unwrap_or_default();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        bail!("{operation} was denied; sign in again");
    }
    bail!("{operation} failed with HTTP {status}: {message}")
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_platform_browser_command_for_https_login() {
        let command = browser_command("https://microsoft.com/link").unwrap();
        #[cfg(target_os = "windows")]
        assert_eq!(command.program, "rundll32");
        #[cfg(target_os = "macos")]
        assert_eq!(command.program, "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(command.program, "xdg-open");
        assert_eq!(
            command.arguments.last().map(String::as_str),
            Some("https://microsoft.com/link")
        );
    }

    #[test]
    fn refuses_non_https_browser_urls() {
        assert!(browser_command("file:///tmp/login").is_err());
        assert!(browser_command("http://example.com").is_err());
    }
}
