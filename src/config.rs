use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Offline,
    Microsoft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub expires_at: u64,
}

impl Account {
    pub fn offline(username: impl Into<String>) -> Self {
        let username = username.into();
        Self {
            id: offline_uuid(&username),
            username,
            kind: AccountKind::Offline,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub java_path: String,
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    pub width: u32,
    pub height: u32,
    pub show_snapshots: bool,
    pub extra_jvm_args: Vec<String>,
    pub microsoft_client_id: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            java_path: "java".to_owned(),
            min_memory_mb: 512,
            max_memory_mb: 4096,
            width: 1280,
            height: 720,
            show_snapshots: false,
            extra_jvm_args: Vec::new(),
            microsoft_client_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub accounts: Vec<Account>,
    pub selected_account: Option<String>,
    pub selected_version: Option<String>,
    pub settings: Settings,
}

impl Default for Config {
    fn default() -> Self {
        let account = Account::offline("Player");
        Self {
            selected_account: Some(account.id.clone()),
            accounts: vec![account],
            selected_version: None,
            settings: Settings::default(),
        }
    }
}

impl Config {
    pub fn rename_offline_account(&mut self, account_id: &str, username: &str) -> Result<Account> {
        let username = username.trim();
        validate_username(username)?;
        let index = self
            .accounts
            .iter()
            .position(|account| account.id == account_id)
            .context("account no longer exists")?;
        if self.accounts[index].kind != AccountKind::Offline {
            bail!("Microsoft account names are managed by the Minecraft profile");
        }
        if self
            .accounts
            .iter()
            .enumerate()
            .any(|(candidate, account)| {
                candidate != index && account.username.eq_ignore_ascii_case(username)
            })
        {
            bail!("An account with this username already exists");
        }

        let renamed = Account::offline(username);
        if self.selected_account.as_deref() == Some(account_id) {
            self.selected_account = Some(renamed.id.clone());
        }
        self.accounts[index] = renamed.clone();
        Ok(renamed)
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    root: PathBuf,
    config_path: PathBuf,
}

impl ConfigStore {
    pub fn discover() -> Result<Self> {
        let root = if let Some(value) = env::var_os("MICRC_HOME") {
            PathBuf::from(value)
        } else if cfg!(target_os = "windows") {
            env::var_os("APPDATA")
                .map(PathBuf::from)
                .context("APPDATA is not set")?
                .join("micrc")
        } else if let Some(value) = env::var_os("XDG_DATA_HOME") {
            PathBuf::from(value).join("micrc")
        } else {
            env::var_os("HOME")
                .map(PathBuf::from)
                .context("HOME is not set")?
                .join(".local/share/micrc")
        };
        Ok(Self::at(root))
    }

    pub fn at(root: PathBuf) -> Self {
        let config_path = root.join("config.json");
        Self { root, config_path }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn load(&self) -> Result<Config> {
        if !self.config_path.exists() {
            return Ok(Config::default());
        }
        let data = fs::read(&self.config_path)
            .with_context(|| format!("failed to read {}", self.config_path.display()))?;
        serde_json::from_slice(&data)
            .with_context(|| format!("failed to parse {}", self.config_path.display()))
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        }
        let data = serde_json::to_vec_pretty(config)?;
        let temporary = self.config_path.with_extension("json.tmp");
        fs::write(&temporary, data)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        #[cfg(windows)]
        let _ = fs::remove_file(&self.config_path);
        fs::rename(&temporary, &self.config_path)
            .with_context(|| format!("failed to replace {}", self.config_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.config_path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

pub fn validate_username(username: &str) -> Result<()> {
    if !(3..=16).contains(&username.len()) {
        bail!("Username must contain between 3 and 16 characters");
    }
    if !username
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        bail!("Username may only contain letters, numbers, and underscores");
    }
    Ok(())
}

fn offline_uuid(username: &str) -> String {
    use md5::{Digest, Md5};

    let digest = Md5::digest(format!("OfflinePlayer:{username}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        u32::from_be_bytes(bytes[0..4].try_into().expect("four bytes")),
        u16::from_be_bytes(bytes[4..6].try_into().expect("two bytes")),
        u16::from_be_bytes(bytes[6..8].try_into().expect("two bytes")),
        u16::from_be_bytes(bytes[8..10].try_into().expect("two bytes")),
        u64::from_be_bytes([
            0, 0, bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_offline_username() {
        assert!(validate_username("Steve_123").is_ok());
        assert!(validate_username("x").is_err());
        assert!(validate_username("not valid").is_err());
    }

    #[test]
    fn creates_standard_offline_uuid() {
        assert_eq!(
            Account::offline("Steve").id,
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
    }

    #[test]
    fn persists_configuration() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let store = ConfigStore::at(temporary.path().join("micrc"));
        let mut config = Config::default();
        config.settings.max_memory_mb = 8192;
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap().settings.max_memory_mb, 8192);
    }

    #[test]
    fn renames_selected_offline_account_and_updates_its_uuid() {
        let mut config = Config::default();
        let old_id = config.accounts[0].id.clone();

        let renamed = config.rename_offline_account(&old_id, "Steve").unwrap();

        assert_eq!(renamed.username, "Steve");
        assert_eq!(renamed.id, "5627dd98-e6be-3c21-b8a8-e92344183641");
        assert_eq!(
            config.selected_account.as_deref(),
            Some(renamed.id.as_str())
        );
        assert_eq!(config.accounts[0].id, renamed.id);
    }

    #[test]
    fn rejects_duplicate_and_microsoft_account_renames() {
        let mut config = Config::default();
        config.accounts.push(Account::offline("Alex"));
        let first_id = config.accounts[0].id.clone();
        assert!(config.rename_offline_account(&first_id, "alex").is_err());

        config.accounts[0].kind = AccountKind::Microsoft;
        assert!(config.rename_offline_account(&first_id, "Steve").is_err());
    }
}
