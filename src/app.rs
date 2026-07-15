use std::{collections::VecDeque, io::Stdout, time::Duration};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::CrosstermBackend};
use reqwest::Client;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    oneshot,
};

use crate::{
    config::{Account, AccountKind, Config, ConfigStore, validate_username},
    minecraft::{
        TaskEvent,
        auth::{login, open_browser, refresh_if_needed},
        install::{
            delete_version, fetch_manifest, install_version, installed_versions, load_metadata,
        },
        launch::{build_launch_command, run_game},
        metadata::{VersionManifest, VersionSummary},
        paths::MinecraftPaths,
    },
    ui,
};

const LOG_LIMIT: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Play,
    Versions,
    Accounts,
    Settings,
    Logs,
}

impl Tab {
    pub const ALL: [Self; 5] = [
        Self::Play,
        Self::Versions,
        Self::Accounts,
        Self::Settings,
        Self::Logs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::Versions => "Versions",
            Self::Accounts => "Accounts",
            Self::Settings => "Settings",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Debug, Clone)]
pub enum InputTarget {
    NewOfflineAccount,
    RenameOfflineAccount(String),
    JavaPath,
    MinMemory,
    MaxMemory,
    Width,
    Height,
    ExtraJvmArgs,
    MicrosoftClientId,
}

#[derive(Debug, Clone)]
pub struct InputModal {
    pub target: InputTarget,
    pub title: String,
    pub value: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    pub browser_status: String,
}

#[derive(Debug, Clone)]
pub struct DeleteVersionConfirmation {
    pub version_id: String,
}

pub struct App {
    pub tab: Tab,
    pub config: Config,
    pub paths: MinecraftPaths,
    pub manifest: Option<VersionManifest>,
    pub installed: Vec<String>,
    pub version_index: usize,
    pub play_version_index: usize,
    pub account_index: usize,
    pub setting_index: usize,
    pub log_scroll: usize,
    pub status: String,
    pub progress: Option<(usize, usize, String)>,
    pub logs: VecDeque<String>,
    pub modal: Option<InputModal>,
    pub version_deletion: Option<DeleteVersionConfirmation>,
    pub device_login: Option<DeviceLogin>,
    pub active_task: bool,
    pub game_running: bool,
    should_quit: bool,
    store: ConfigStore,
    client: Client,
    events_tx: UnboundedSender<TaskEvent>,
    events_rx: UnboundedReceiver<TaskEvent>,
    auth_cancel: Option<oneshot::Sender<()>>,
}

impl App {
    pub async fn load(store: ConfigStore) -> Result<Self> {
        let config = store.load()?;
        let paths = MinecraftPaths::new(store.root().join("minecraft"));
        let installed = installed_versions(&paths).await?;
        let play_version_index = config
            .selected_version
            .as_ref()
            .and_then(|selected| installed.iter().position(|id| id == selected))
            .unwrap_or_default();
        let account_index = config
            .selected_account
            .as_ref()
            .and_then(|selected| config.accounts.iter().position(|item| &item.id == selected))
            .unwrap_or_default();
        let (events_tx, events_rx) = unbounded_channel();
        let client = Client::builder()
            .user_agent(format!("micrc/{}", env!("CARGO_PKG_VERSION")))
            .build()?;
        let mut app = Self {
            tab: Tab::Play,
            config,
            paths,
            manifest: None,
            installed,
            version_index: 0,
            play_version_index,
            account_index,
            setting_index: 0,
            log_scroll: 0,
            status: "Loading version catalog".to_owned(),
            progress: None,
            logs: VecDeque::new(),
            modal: None,
            version_deletion: None,
            device_login: None,
            active_task: false,
            game_running: false,
            should_quit: false,
            store,
            client,
            events_tx,
            events_rx,
            auth_cancel: None,
        };
        app.refresh_manifest();
        Ok(app)
    }

    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| ui::draw(frame, self))?;
            self.drain_task_events().await;
            if event::poll(Duration::from_millis(75))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key).await?;
            }
        }
        self.store.save(&self.config)?;
        Ok(())
    }

    pub fn visible_versions(&self) -> Vec<&VersionSummary> {
        self.manifest
            .as_ref()
            .map(|manifest| {
                manifest
                    .versions
                    .iter()
                    .filter(|version| {
                        self.config.settings.show_snapshots || version.kind == "release"
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn refresh_manifest(&mut self) {
        if self.active_task {
            self.status = "Another task is already running".to_owned();
            return;
        }
        self.active_task = true;
        self.progress = None;
        self.status = "Refreshing version catalog".to_owned();
        let client = self.client.clone();
        let events = self.events_tx.clone();
        tokio::spawn(async move {
            match fetch_manifest(&client).await {
                Ok(manifest) => {
                    events.send(TaskEvent::ManifestLoaded(manifest)).ok();
                }
                Err(error) => {
                    events.send(TaskEvent::Failed(error.to_string())).ok();
                }
            }
        });
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.device_login.is_some() {
            match key.code {
                KeyCode::Esc => {
                    if let Some(cancel) = self.auth_cancel.take() {
                        cancel.send(()).ok();
                    }
                    self.device_login = None;
                    self.status = "Cancelling Microsoft login".to_owned();
                }
                KeyCode::Enter | KeyCode::Char('o') => {
                    let url = self
                        .device_login
                        .as_ref()
                        .map(|login| login.verification_uri.clone())
                        .unwrap_or_default();
                    let result = open_browser(&url).await;
                    let browser_status = match &result {
                        Ok(()) => "Browser opened. Complete the sign-in there.".to_owned(),
                        Err(error) => format!("Could not open the browser: {error}"),
                    };
                    if let Some(login) = &mut self.device_login {
                        login.browser_status = browser_status;
                    }
                    self.status = if result.is_ok() {
                        "Waiting for Microsoft authorization".to_owned()
                    } else {
                        "Use the displayed URL and code to sign in".to_owned()
                    };
                }
                _ => {}
            }
            return Ok(());
        }
        if self.version_deletion.is_some() {
            self.handle_version_deletion_key(key);
            return Ok(());
        }
        if self.modal.is_some() {
            return self.handle_modal_key(key);
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => self.move_tab(1),
            KeyCode::BackTab => self.move_tab(-1),
            _ => match self.tab {
                Tab::Play => self.handle_play_key(key).await?,
                Tab::Versions => self.handle_versions_key(key),
                Tab::Accounts => self.handle_accounts_key(key),
                Tab::Settings => self.handle_settings_key(key),
                Tab::Logs => self.handle_logs_key(key),
            },
        }
        Ok(())
    }

    fn move_tab(&mut self, direction: isize) {
        let current = Tab::ALL
            .iter()
            .position(|tab| tab == &self.tab)
            .unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(Tab::ALL.len() as isize) as usize;
        self.tab = Tab::ALL[next];
    }

    async fn handle_play_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.play_version_index = previous(self.play_version_index, self.installed.len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.play_version_index = next(self.play_version_index, self.installed.len())
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.account_index = previous(self.account_index, self.config.accounts.len())
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.account_index = next(self.account_index, self.config.accounts.len())
            }
            KeyCode::Enter => self.launch_selected().await?,
            KeyCode::Delete | KeyCode::Char('d') => {
                if let Some(version) = self.installed.get(self.play_version_index).cloned() {
                    self.request_version_deletion(version);
                } else {
                    self.status = "No installed version is selected".to_owned();
                }
            }
            _ => {}
        }
        self.update_selections()?;
        Ok(())
    }

    fn handle_versions_key(&mut self, key: KeyEvent) {
        let len = self.visible_versions().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.version_index = previous(self.version_index, len)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.version_index = next(self.version_index, len)
            }
            KeyCode::Enter | KeyCode::Char('i') => self.install_selected(),
            KeyCode::Delete | KeyCode::Char('d') => {
                let version = self
                    .visible_versions()
                    .get(self.version_index)
                    .map(|version| version.id.clone());
                if let Some(version) = version {
                    self.request_version_deletion(version);
                } else {
                    self.status = "No version is selected".to_owned();
                }
            }
            KeyCode::Char('r') => self.refresh_manifest(),
            KeyCode::Char('s') => {
                self.config.settings.show_snapshots = !self.config.settings.show_snapshots;
                self.version_index = 0;
                self.save_config();
            }
            _ => {}
        }
    }

    fn handle_accounts_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.account_index = previous(self.account_index, self.config.accounts.len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.account_index = next(self.account_index, self.config.accounts.len())
            }
            KeyCode::Enter => {
                if let Some(account) = self.config.accounts.get(self.account_index) {
                    self.config.selected_account = Some(account.id.clone());
                    self.status = format!("Selected account {}", account.username);
                    self.save_config();
                }
            }
            KeyCode::Char('n') => self.open_input(
                InputTarget::NewOfflineAccount,
                "New offline account",
                String::new(),
            ),
            KeyCode::Char('e') => self.edit_offline_account(),
            KeyCode::Char('m') => self.start_microsoft_login(),
            KeyCode::Delete | KeyCode::Char('d') => self.delete_account(),
            _ => {}
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        const COUNT: usize = 8;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.setting_index = previous(self.setting_index, COUNT)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.setting_index = next(self.setting_index, COUNT)
            }
            KeyCode::Enter => self.edit_setting(),
            KeyCode::Char(' ') if self.setting_index == 7 => {
                self.config.settings.show_snapshots = !self.config.settings.show_snapshots;
                self.save_config();
            }
            _ => {}
        }
    }

    fn handle_logs_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.log_scroll = self.log_scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.log_scroll = self.log_scroll.saturating_add(1).min(self.logs.len())
            }
            KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_sub(10),
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_add(10).min(self.logs.len())
            }
            KeyCode::Home => self.log_scroll = 0,
            KeyCode::End => self.log_scroll = self.logs.len().saturating_sub(1),
            KeyCode::Char('c') => {
                self.logs.clear();
                self.log_scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => self.modal = None,
            KeyCode::Enter => self.submit_modal()?,
            KeyCode::Backspace => {
                if let Some(modal) = &mut self.modal {
                    modal.value.pop();
                    modal.error = None;
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(modal) = &mut self.modal {
                    modal.value.push(character);
                    modal.error = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn open_input(&mut self, target: InputTarget, title: &str, value: String) {
        self.modal = Some(InputModal {
            target,
            title: title.to_owned(),
            value,
            error: None,
        });
    }

    fn submit_modal(&mut self) -> Result<()> {
        let Some(modal) = self.modal.clone() else {
            return Ok(());
        };
        let result: Result<()> =
            (|| {
                match modal.target {
                    InputTarget::NewOfflineAccount => {
                        validate_username(modal.value.trim())?;
                        if self.config.accounts.iter().any(|account| {
                            account.username.eq_ignore_ascii_case(modal.value.trim())
                        }) {
                            anyhow::bail!("An account with this username already exists");
                        }
                        let account = Account::offline(modal.value.trim());
                        self.config.selected_account = Some(account.id.clone());
                        self.config.accounts.push(account);
                        self.account_index = self.config.accounts.len() - 1;
                        self.status = "Offline account added".to_owned();
                    }
                    InputTarget::RenameOfflineAccount(account_id) => {
                        let renamed = self
                            .config
                            .rename_offline_account(&account_id, &modal.value)?;
                        self.account_index = self
                            .config
                            .accounts
                            .iter()
                            .position(|account| account.id == renamed.id)
                            .unwrap_or_default();
                        self.status = format!("Renamed offline account to {}", renamed.username);
                    }
                    InputTarget::JavaPath => {
                        if modal.value.trim().is_empty() {
                            anyhow::bail!("Java path cannot be empty");
                        }
                        self.config.settings.java_path = modal.value.trim().to_owned();
                    }
                    InputTarget::MinMemory => {
                        let value = parse_positive(&modal.value, "minimum memory")?;
                        if value > self.config.settings.max_memory_mb {
                            anyhow::bail!("Minimum memory cannot exceed maximum memory");
                        }
                        self.config.settings.min_memory_mb = value;
                    }
                    InputTarget::MaxMemory => {
                        let value = parse_positive(&modal.value, "maximum memory")?;
                        if value < self.config.settings.min_memory_mb {
                            anyhow::bail!("Maximum memory cannot be lower than minimum memory");
                        }
                        self.config.settings.max_memory_mb = value;
                    }
                    InputTarget::Width => {
                        self.config.settings.width = parse_positive(&modal.value, "window width")?;
                    }
                    InputTarget::Height => {
                        self.config.settings.height =
                            parse_positive(&modal.value, "window height")?;
                    }
                    InputTarget::ExtraJvmArgs => {
                        self.config.settings.extra_jvm_args =
                            modal.value.split_whitespace().map(str::to_owned).collect();
                    }
                    InputTarget::MicrosoftClientId => {
                        self.config.settings.microsoft_client_id = modal.value.trim().to_owned();
                    }
                }
                Ok(())
            })();
        match result {
            Ok(()) => {
                self.modal = None;
                self.save_config();
            }
            Err(error) => {
                if let Some(active) = &mut self.modal {
                    active.error = Some(error.to_string());
                }
            }
        }
        Ok(())
    }

    fn edit_setting(&mut self) {
        match self.setting_index {
            0 => self.open_input(
                InputTarget::JavaPath,
                "Java executable",
                self.config.settings.java_path.clone(),
            ),
            1 => self.open_input(
                InputTarget::MinMemory,
                "Minimum memory (MB)",
                self.config.settings.min_memory_mb.to_string(),
            ),
            2 => self.open_input(
                InputTarget::MaxMemory,
                "Maximum memory (MB)",
                self.config.settings.max_memory_mb.to_string(),
            ),
            3 => self.open_input(
                InputTarget::Width,
                "Window width",
                self.config.settings.width.to_string(),
            ),
            4 => self.open_input(
                InputTarget::Height,
                "Window height",
                self.config.settings.height.to_string(),
            ),
            5 => self.open_input(
                InputTarget::ExtraJvmArgs,
                "Extra JVM arguments",
                self.config.settings.extra_jvm_args.join(" "),
            ),
            6 => self.open_input(
                InputTarget::MicrosoftClientId,
                "Microsoft application client ID",
                self.config.settings.microsoft_client_id.clone(),
            ),
            7 => {
                self.config.settings.show_snapshots = !self.config.settings.show_snapshots;
                self.save_config();
            }
            _ => {}
        }
    }

    fn edit_offline_account(&mut self) {
        let Some(account) = self.config.accounts.get(self.account_index).cloned() else {
            self.status = "No account is selected".to_owned();
            return;
        };
        if account.kind != AccountKind::Offline {
            self.status = "Microsoft account names are managed by the Minecraft profile".to_owned();
            return;
        }
        self.open_input(
            InputTarget::RenameOfflineAccount(account.id),
            "Offline account name",
            account.username,
        );
    }

    fn request_version_deletion(&mut self, version_id: String) {
        if self.active_task || self.game_running {
            self.status = "Minecraft or another task is already running".to_owned();
            return;
        }
        if !self
            .installed
            .iter()
            .any(|installed| installed == &version_id)
        {
            self.status = format!("Minecraft {version_id} is not installed");
            return;
        }
        self.version_deletion = Some(DeleteVersionConfirmation { version_id });
    }

    fn handle_version_deletion_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.version_deletion = None;
                self.status = "Version deletion cancelled".to_owned();
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Some(confirmation) = self.version_deletion.take() {
                    self.delete_version(confirmation.version_id);
                }
            }
            _ => {}
        }
    }

    fn delete_version(&mut self, version_id: String) {
        self.active_task = true;
        self.progress = None;
        self.status = format!("Deleting Minecraft {version_id}");
        let paths = self.paths.clone();
        let events = self.events_tx.clone();
        tokio::spawn(async move {
            match delete_version(&paths, &version_id).await {
                Ok(()) => {
                    events.send(TaskEvent::Deleted(version_id)).ok();
                }
                Err(error) => {
                    events.send(TaskEvent::Failed(error.to_string())).ok();
                }
            }
        });
    }

    fn install_selected(&mut self) {
        if self.active_task || self.game_running {
            self.status = "Another task is already running".to_owned();
            return;
        }
        let version = self
            .visible_versions()
            .get(self.version_index)
            .cloned()
            .cloned();
        let Some(version) = version else {
            self.status = "No version is selected".to_owned();
            return;
        };
        self.active_task = true;
        self.progress = None;
        let client = self.client.clone();
        let paths = self.paths.clone();
        let events = self.events_tx.clone();
        tokio::spawn(async move {
            if let Err(error) = install_version(client, paths, version, events.clone()).await {
                events.send(TaskEvent::Failed(error.to_string())).ok();
            }
        });
    }

    fn start_microsoft_login(&mut self) {
        if self.active_task || self.game_running {
            self.status = "Another task is already running".to_owned();
            return;
        }
        let client_id = std::env::var("MICRC_MICROSOFT_CLIENT_ID")
            .unwrap_or_else(|_| self.config.settings.microsoft_client_id.clone());
        if client_id.trim().is_empty() {
            self.status = "Set a Microsoft application client ID in Settings first".to_owned();
            self.tab = Tab::Settings;
            self.setting_index = 6;
            return;
        }
        self.active_task = true;
        self.status = "Starting Microsoft login".to_owned();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.auth_cancel = Some(cancel_tx);
        let client = self.client.clone();
        let events = self.events_tx.clone();
        tokio::spawn(async move {
            match login(client, client_id, events.clone(), cancel_rx).await {
                Ok(account) => {
                    events.send(TaskEvent::Authenticated(account)).ok();
                }
                Err(error) => {
                    events.send(TaskEvent::Failed(error.to_string())).ok();
                }
            }
        });
    }

    async fn launch_selected(&mut self) -> Result<()> {
        if self.active_task || self.game_running {
            self.status = "Minecraft or another task is already running".to_owned();
            return Ok(());
        }
        let Some(version) = self.installed.get(self.play_version_index).cloned() else {
            self.status = "Install a Minecraft version first".to_owned();
            return Ok(());
        };
        let Some(account) = self.config.accounts.get(self.account_index).cloned() else {
            self.status = "Add an account first".to_owned();
            return Ok(());
        };
        let metadata = load_metadata(&self.paths, &version).await?;
        self.game_running = true;
        self.status = format!("Starting {version}");
        self.tab = Tab::Logs;
        let client = self.client.clone();
        let client_id = std::env::var("MICRC_MICROSOFT_CLIENT_ID")
            .unwrap_or_else(|_| self.config.settings.microsoft_client_id.clone());
        let settings = self.config.settings.clone();
        let paths = self.paths.clone();
        let events = self.events_tx.clone();
        tokio::spawn(async move {
            let result = async {
                let refreshed = refresh_if_needed(&client, &client_id, &account).await?;
                if refreshed.access_token != account.access_token {
                    events
                        .send(TaskEvent::AccountUpdated(refreshed.clone()))
                        .ok();
                }
                let required_java = metadata
                    .java_version
                    .as_ref()
                    .map(|java| java.major_version);
                let command = build_launch_command(&metadata, &refreshed, &settings, &paths)?;
                run_game(command, required_java, events.clone()).await
            }
            .await;
            if let Err(error) = result {
                events.send(TaskEvent::Failed(error.to_string())).ok();
            }
        });
        Ok(())
    }

    fn delete_account(&mut self) {
        if self.config.accounts.len() <= 1 {
            self.status = "At least one account must remain".to_owned();
            return;
        }
        let removed = self.config.accounts.remove(self.account_index);
        self.account_index = self.account_index.min(self.config.accounts.len() - 1);
        let account = &self.config.accounts[self.account_index];
        self.config.selected_account = Some(account.id.clone());
        self.status = format!("Removed account {}", removed.username);
        self.save_config();
    }

    fn update_selections(&mut self) -> Result<()> {
        if let Some(version) = self.installed.get(self.play_version_index) {
            self.config.selected_version = Some(version.clone());
        }
        if let Some(account) = self.config.accounts.get(self.account_index) {
            self.config.selected_account = Some(account.id.clone());
        }
        self.store.save(&self.config)
    }

    fn save_config(&mut self) {
        if let Err(error) = self.store.save(&self.config) {
            self.status = format!("Failed to save configuration: {error}");
        }
    }

    async fn drain_task_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                TaskEvent::ManifestLoaded(manifest) => {
                    let count = manifest.versions.len();
                    self.manifest = Some(manifest);
                    self.version_index = 0;
                    self.active_task = false;
                    self.status = format!("Loaded {count} Minecraft versions");
                }
                TaskEvent::Status(status) => self.status = status,
                TaskEvent::Progress {
                    completed,
                    total,
                    label,
                } => {
                    self.status = format!("Downloading {label}");
                    self.progress = Some((completed, total, label));
                }
                TaskEvent::Log(line) => {
                    if self.logs.len() >= LOG_LIMIT {
                        self.logs.pop_front();
                    }
                    self.logs.push_back(line);
                    self.log_scroll = self.logs.len().saturating_sub(1);
                }
                TaskEvent::DeviceCode {
                    user_code,
                    verification_uri,
                    message,
                    browser_opened,
                    browser_error,
                } => {
                    let browser_status = if browser_opened {
                        "Browser opened. Complete the sign-in there.".to_owned()
                    } else {
                        format!(
                            "Could not open the browser: {}",
                            browser_error.unwrap_or_else(|| "unknown error".to_owned())
                        )
                    };
                    self.device_login = Some(DeviceLogin {
                        user_code,
                        verification_uri,
                        message,
                        browser_status,
                    });
                    self.status = if browser_opened {
                        "Waiting for Microsoft authorization".to_owned()
                    } else {
                        "Use the displayed URL and code to sign in".to_owned()
                    };
                }
                TaskEvent::Authenticated(account) => {
                    self.active_task = false;
                    self.auth_cancel = None;
                    self.device_login = None;
                    if let Some(existing) = self
                        .config
                        .accounts
                        .iter_mut()
                        .find(|existing| existing.id == account.id)
                    {
                        *existing = account.clone();
                    } else {
                        self.config.accounts.push(account.clone());
                    }
                    self.account_index = self
                        .config
                        .accounts
                        .iter()
                        .position(|existing| existing.id == account.id)
                        .unwrap_or_default();
                    self.config.selected_account = Some(account.id);
                    self.status = format!("Signed in as {}", account.username);
                    self.save_config();
                }
                TaskEvent::AccountUpdated(account) => {
                    if let Some(existing) = self
                        .config
                        .accounts
                        .iter_mut()
                        .find(|existing| existing.id == account.id)
                    {
                        *existing = account;
                        self.save_config();
                    }
                }
                TaskEvent::Installed(version) => {
                    self.active_task = false;
                    self.progress = None;
                    self.status = format!("Installed Minecraft {version}");
                    match installed_versions(&self.paths).await {
                        Ok(installed) => {
                            self.installed = installed;
                            self.play_version_index = self
                                .installed
                                .iter()
                                .position(|id| id == &version)
                                .unwrap_or_default();
                            self.config.selected_version = Some(version);
                            self.save_config();
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
                TaskEvent::Deleted(version) => {
                    self.active_task = false;
                    self.progress = None;
                    let selected = self.config.selected_version.clone();
                    match installed_versions(&self.paths).await {
                        Ok(installed) => {
                            self.installed = installed;
                            self.play_version_index = selected
                                .as_ref()
                                .and_then(|id| {
                                    self.installed.iter().position(|installed| installed == id)
                                })
                                .unwrap_or_else(|| {
                                    self.play_version_index
                                        .min(self.installed.len().saturating_sub(1))
                                });
                            self.config.selected_version =
                                self.installed.get(self.play_version_index).cloned();
                            self.status = format!("Deleted Minecraft {version}");
                            self.save_config();
                        }
                        Err(error) => self.status = format!("Error: {error}"),
                    }
                }
                TaskEvent::GameExited(code) => {
                    self.game_running = false;
                    self.status = match code {
                        Some(0) => "Minecraft exited normally".to_owned(),
                        Some(code) => format!("Minecraft exited with code {code}"),
                        None => "Minecraft was terminated".to_owned(),
                    };
                }
                TaskEvent::Failed(error) => {
                    self.active_task = false;
                    self.game_running = false;
                    self.progress = None;
                    self.device_login = None;
                    self.auth_cancel = None;
                    self.status = format!("Error: {error}");
                    if self.logs.len() >= LOG_LIMIT {
                        self.logs.pop_front();
                    }
                    self.logs.push_back(format!("[micrc] {error}"));
                }
            }
        }
    }
}

fn previous(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else if index == 0 {
        len - 1
    } else {
        index - 1
    }
}

fn next(index: usize, len: usize) -> usize {
    if len == 0 { 0 } else { (index + 1) % len }
}

fn parse_positive(value: &str, label: &str) -> Result<u32> {
    let parsed = value
        .trim()
        .parse::<u32>()
        .with_context(|| format!("Invalid {label}"))?;
    if parsed == 0 {
        anyhow::bail!("{label} must be greater than zero");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn edits_an_offline_account_name_through_the_account_modal() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let mut app = App::load(ConfigStore::at(temporary.path().join("micrc")))
            .await
            .unwrap();

        app.handle_accounts_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        let modal = app.modal.as_mut().expect("rename modal");
        assert!(matches!(modal.target, InputTarget::RenameOfflineAccount(_)));
        modal.value = "Steve".to_owned();
        app.submit_modal().unwrap();

        assert_eq!(app.config.accounts[0].username, "Steve");
        assert_eq!(
            app.config.selected_account.as_deref(),
            Some("5627dd98-e6be-3c21-b8a8-e92344183641")
        );
        assert!(app.modal.is_none());
    }

    #[tokio::test]
    async fn confirms_and_applies_version_deletion() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let mut app = App::load(ConfigStore::at(temporary.path().join("micrc")))
            .await
            .unwrap();
        let (events_tx, events_rx) = unbounded_channel();
        app.events_tx = events_tx;
        app.events_rx = events_rx;
        let version_id = "test-version";
        tokio::fs::create_dir_all(app.paths.version_dir(version_id))
            .await
            .unwrap();
        tokio::fs::write(app.paths.version_json(version_id), b"{}")
            .await
            .unwrap();
        tokio::fs::write(app.paths.version_jar(version_id), b"jar")
            .await
            .unwrap();
        app.active_task = false;
        app.installed = vec![version_id.to_owned()];
        app.config.selected_version = Some(version_id.to_owned());

        app.handle_play_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .await
            .unwrap();
        assert_eq!(
            app.version_deletion
                .as_ref()
                .map(|value| value.version_id.as_str()),
            Some(version_id)
        );
        app.handle_version_deletion_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let deleted_event = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(event) = app.events_rx.recv().await
                    && matches!(&event, TaskEvent::Deleted(id) if id == version_id)
                {
                    break event;
                }
            }
        })
        .await
        .expect("delete event");
        app.events_tx.send(deleted_event).unwrap();
        app.drain_task_events().await;

        assert!(!app.paths.version_dir(version_id).exists());
        assert!(app.installed.is_empty());
        assert!(app.config.selected_version.is_none());
        assert_eq!(app.status, "Deleted Minecraft test-version");
    }
}
