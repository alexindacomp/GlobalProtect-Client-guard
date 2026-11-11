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

// ---------- автозапуск ----------
//
// Два независимых механизма, чтобы пропуск одного не оставлял пользователя без VPN:
// 1. задача Планировщика с триггером «при входе этого пользователя» (с задержкой,
//    чтобы панель задач успела появиться) — основной;
// 2. значение HKCU\...\Run — запасной.
// Повторный запуск безопасен: второй экземпляр с `--minimized` молча завершается.

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "GpGuard";
const TASK_NAME: &str = "GP Guard";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Фактическое состояние автозапуска (для отображения в настройках).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutostartState {
    pub task: bool,
    pub registry: bool,
}

impl AutostartState {
    pub fn any(self) -> bool {
        self.task || self.registry
    }
}

fn autostart_command() -> Result<(String, String), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok((exe.display().to_string(), "--minimized".to_string()))
}

fn schtasks(args: &[&str]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn current_user() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    if domain.is_empty() { user } else { format!("{domain}\\{user}") }
}

fn task_xml(exe: &str, args: &str) -> String {
    let user = xml_escape(&current_user());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>GP Guard autostart</Description></RegistrationInfo>
  <Triggers>
    <LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId><Delay>PT15S</Delay></LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author">
    <Exec><Command>{exe}</Command><Arguments>{args}</Arguments></Exec>
  </Actions>
</Task>
"#,
        exe = xml_escape(exe),
        args = xml_escape(args),
    )
}

/// Определение задачи, если она есть (вывод `schtasks /query /xml`).
fn query_task_xml() -> Option<String> {
    let out = schtasks(&["/query", "/tn", TASK_NAME, "/xml"]).ok()?;
    if !out.status.success() {
        return None;
    }
    // schtasks выводит XML в кодировке консоли; путь к exe сравниваем без учёта регистра
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn task_matches(exe: &str) -> bool {
    query_task_xml()
        .map(|x| x.to_lowercase().contains(&xml_escape(exe).to_lowercase()))
        .unwrap_or(false)
}

fn register_task(exe: &str, args: &str) -> Result<(), String> {
    // XML-файл в UTF-16 LE с BOM — формат, который ожидает schtasks /xml
    let path = app_dir().join("autostart-task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    for u in task_xml(exe, args).encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let path_s = path.display().to_string();
    let out = schtasks(&["/create", "/tn", TASK_NAME, "/xml", &path_s, "/f"]);
    let _ = std::fs::remove_file(&path);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn delete_task() -> Result<(), String> {
    if query_task_xml().is_none() {
        return Ok(());
    }
    let out = schtasks(&["/delete", "/tn", TASK_NAME, "/f"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn run_value_expected() -> Result<String, String> {
    let (exe, args) = autostart_command()?;
    Ok(format!("\"{exe}\" {args}"))
}

fn set_run_value(enabled: bool) -> Result<(), String> {
    use winreg::{enums::*, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN_KEY)
        .map_err(|e| e.to_string())?;
    if enabled {
        let want = run_value_expected()?;
        // перезаписываем только при отличии, чтобы не трогать реестр на каждом старте
        if key.get_value::<String, _>(RUN_VALUE).ok().as_deref() != Some(want.as_str()) {
            key.set_value(RUN_VALUE, &want).map_err(|e| e.to_string())?;
        }
        Ok(())
    } else {
        match key.delete_value(RUN_VALUE) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Включить/выключить автозапуск. При включении приводит задачу и значение Run
/// к текущему пути exe (exe могли переместить). Ошибка — только если не удалось
/// настроить ни один из механизмов (или убрать хотя бы один при выключении).
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let run = set_run_value(enabled);
    let task = if enabled {
        let (exe, args) = autostart_command()?;
        if task_matches(&exe) { Ok(()) } else { register_task(&exe, &args) }
    } else {
        delete_task()
    };
    match (enabled, run, task) {
        (_, Ok(()), Ok(())) => Ok(()),
        // включение: достаточно одного работающего механизма
        (true, Ok(()), Err(_)) | (true, Err(_), Ok(())) => Ok(()),
        (_, Err(a), Err(b)) => Err(format!("{a}; {b}")),
        (false, Err(e), _) | (false, _, Err(e)) => Err(e),
    }
}

/// Фактическое состояние: есть ли задача Планировщика и значение Run для этого exe.
pub fn autostart_state() -> AutostartState {
    use winreg::{enums::*, RegKey};
    let registry = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE).ok())
        .zip(run_value_expected().ok())
        .is_some_and(|(have, want)| have.eq_ignore_ascii_case(&want));
    let task = autostart_command().map(|(exe, _)| task_matches(&exe)).unwrap_or(false);
    AutostartState { task, registry }
}
