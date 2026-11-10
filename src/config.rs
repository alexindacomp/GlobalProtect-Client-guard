//! Профиль пользователя и настройки. Пароль хранится отдельно — в
//! Windows Credential Manager (DPAPI), а не в JSON-файле.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "gp-guard";
pub const DEFAULT_GP_PATH: &str = r"C:\Program Files\Palo Alto Networks\GlobalProtect\PanGPA.exe";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Адрес портала GlobalProtect (вводится, только если GP его спросит).
    pub portal: String,
    pub username: String,
    /// Путь к PanGPA.exe.
    pub gp_path: String,
    /// Следить за подключением и переподключать при обрыве.
    pub auto_reconnect: bool,
    /// Подключаться при старте приложения, если VPN не поднят.
    pub connect_on_start: bool,
    /// Период проверки состояния, сек.
    pub check_interval_secs: u64,
    /// Сколько ждать после обрыва, прежде чем вмешиваться (GP может переподключиться сам), сек.
    pub grace_secs: u64,
    /// Пауза между попытками подключения, сек.
    pub retry_interval_secs: u64,
    /// После скольких неудачных попыток перезапускать PanGPA.exe.
    pub attempts_before_restart: u32,
    /// Необязательная проверка доступности внутреннего ресурса "host:port".
    /// Если задана — VPN считается живым только когда TCP-подключение проходит.
    pub health_check: String,
    pub autostart: bool,
    pub start_minimized: bool,
    /// Язык интерфейса: "auto" (как в Windows), "ru", "uk" или "en".
    pub language: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            portal: detect_portal().unwrap_or_default(),
            username: String::new(),
            gp_path: DEFAULT_GP_PATH.to_string(),
            auto_reconnect: true,
            connect_on_start: true,
            check_interval_secs: 5,
            grace_secs: 20,
            retry_interval_secs: 60,
            attempts_before_restart: 2,
            health_check: String::new(),
            autostart: true,
            start_minimized: true,
            language: "auto".to_string(),
        }
    }
}

pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("GpGuard");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("config serializes");
        std::fs::write(config_path(), json)
    }

    pub fn is_profile_complete(&self) -> bool {
        !self.username.is_empty() && load_password(&self.username).is_some()
    }
}

/// Портал, который уже прописан в установке GP (HKLM\...\PanSetup\Portal).
fn detect_portal() -> Option<String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Palo Alto Networks\GlobalProtect\PanSetup")
        .ok()?;
    let portal: String = key.get_value("Portal").ok()?;
    (!portal.is_empty()).then_some(portal)
}

pub fn load_password(username: &str) -> Option<String> {
    if username.is_empty() {
        return None;
    }
    keyring::Entry::new(KEYRING_SERVICE, username).ok()?.get_password().ok()
}

pub fn store_password(username: &str, password: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, username).map_err(|e| e.to_string())?;
    entry.set_password(password).map_err(|e| e.to_string())
}

pub fn delete_password(username: &str) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, username) {
        let _ = e.delete_credential();
    }
}

// ---------- автозапуск (HKCU\...\Run) ----------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "GpGuard";

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    use winreg::{enums::*, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN_KEY)
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value(RUN_VALUE, &format!("\"{}\" --minimized", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value(RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
