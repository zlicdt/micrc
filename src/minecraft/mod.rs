pub mod auth;
pub mod download;
pub mod install;
pub mod launch;
pub mod metadata;
pub mod paths;

pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone)]
pub enum TaskEvent {
    ManifestLoaded(metadata::VersionManifest),
    Status(String),
    Progress {
        completed: usize,
        total: usize,
        label: String,
    },
    Log(String),
    DeviceCode {
        user_code: String,
        verification_uri: String,
        message: String,
        browser_opened: bool,
        browser_error: Option<String>,
    },
    Authenticated(crate::config::Account),
    AccountUpdated(crate::config::Account),
    Installed(String),
    Deleted(String),
    GameExited(Option<i32>),
    Failed(String),
}
