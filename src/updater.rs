//! Обновление из GitHub Releases: проверка последней версии, загрузка
//! `gp-guard.exe`, проверка целостности и замена собственного exe с перезапуском.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::i18n;

const REPO: &str = "alexindacomp/GlobalProtect-Client-guard";
const ASSET_NAME: &str = "gp-guard.exe";
/// Защита от неожиданно большого ответа: exe весит несколько МБ.
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Аргумент, с которым новый exe запускается после замены.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    asset_url: String,
    asset_size: u64,
    /// "sha256:<hex>" из API GitHub, если есть.
    asset_digest: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing,
    Error(String),
}

fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// `true`, если `candidate` новее текущей версии приложения.
pub fn is_newer(candidate: &str) -> bool {
    match (parse_version(candidate), parse_version(CURRENT_VERSION)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .user_agent(&format!("gp-guard/{CURRENT_VERSION}"))
        .build()
}

/// Последний релиз на GitHub; `Ok(None)`, если он не новее текущей версии.
pub fn check() -> Result<Option<Release>, String> {
    let rel = fetch_latest()?;
    Ok(is_newer(&rel.version).then_some(rel))
}

/// Последний релиз на GitHub без сравнения версий.
fn fetch_latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(ASSET_NAME)))
        .ok_or_else(|| i18n::update_no_asset(&tag))?;
    Ok(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        page_url: v["html_url"].as_str().unwrap_or_default().to_string(),
        asset_url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        asset_size: asset["size"].as_u64().unwrap_or(0),
        asset_digest: asset["digest"].as_str().map(str::to_string),
    })
}

fn download(rel: &Release) -> Result<Vec<u8>, String> {
    if !rel.asset_url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) {
        return Err(i18n::update_bad_source(&rel.asset_url));
    }
    let resp = agent().get(&rel.asset_url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(rel.asset_size as usize);
    resp.into_reader()
        .take(MAX_ASSET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err(i18n::update_verify_failed("size limit"));
    }
    if rel.asset_size != 0 && bytes.len() as u64 != rel.asset_size {
        return Err(i18n::update_verify_failed(&format!("size {} != {}", bytes.len(), rel.asset_size)));
    }
    if let Some(expected) = rel.asset_digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(i18n::update_verify_failed("sha256"));
        }
    }
    if !bytes.starts_with(b"MZ") {
        return Err(i18n::update_verify_failed("not a Windows executable"));
    }
    Ok(bytes)
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Скачать, проверить и подменить собственный exe, затем запустить новую версию.
/// После `Ok(())` вызывающий должен завершить процесс: новый экземпляр ждёт,
/// пока этот освободит мьютекс единственного экземпляра.
pub fn install(rel: &Release, log: &dyn Fn(String)) -> Result<(), String> {
    log(i18n::log_update_downloading(&rel.version));
    let bytes = download(rel)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    swap_in(&exe, &bytes)?;
    if let Err(e) = std::process::Command::new(&exe).arg(AFTER_UPDATE_ARG).spawn() {
        // новый exe не стартует — возвращаем старый, чтобы не остаться без приложения
        roll_back(&exe);
        return Err(e.to_string());
    }
    log(i18n::log_update_restarting(&rel.version));
    Ok(())
}

/// Поставить `bytes` на место `exe`, сохранив прежний файл как `<exe>.old`.
/// Запущенный exe нельзя перезаписать, но можно переименовать.
fn swap_in(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let new_path = sibling(exe, ".new");
    let old_path = sibling(exe, ".old");
    std::fs::write(&new_path, bytes).map_err(|e| i18n::update_write_failed(&e.to_string()))?;
    let _ = std::fs::remove_file(&old_path);
    if let Err(e) = std::fs::rename(exe, &old_path) {
        let _ = std::fs::remove_file(&new_path);
        return Err(i18n::update_write_failed(&e.to_string()));
    }
    if let Err(e) = std::fs::rename(&new_path, exe) {
        let _ = std::fs::rename(&old_path, exe); // откат
        return Err(i18n::update_write_failed(&e.to_string()));
    }
    Ok(())
}

/// Вернуть прежний exe после неудачного запуска новой версии.
fn roll_back(exe: &Path) {
    let new_path = sibling(exe, ".new");
    let old_path = sibling(exe, ".old");
    if std::fs::rename(exe, &new_path).is_ok() && std::fs::rename(&old_path, exe).is_ok() {
        let _ = std::fs::remove_file(&new_path);
    }
}

/// Удалить exe предыдущей версии, оставшийся после обновления (старый процесс
/// может ещё завершаться — поэтому несколько попыток).
pub fn cleanup_old_exe() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = sibling(&exe, ".old");
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// ---------------------------------------------------------------- фоновые операции

use crate::config::Config;
use crate::shared::SharedRef;
use std::sync::{Arc, Mutex};

fn set_state(shared: &SharedRef, s: UpdateState) {
    *shared.update.lock().unwrap() = s;
    shared.request_repaint();
}

/// Проверка в фоне. `manual` — пользователь нажал кнопку (ошибку показываем всегда).
pub fn spawn_check(shared: SharedRef, manual: bool) {
    {
        let mut st = shared.update.lock().unwrap();
        if matches!(*st, UpdateState::Checking | UpdateState::Installing) {
            return;
        }
        *st = UpdateState::Checking;
    }
    shared.request_repaint();
    std::thread::spawn(move || match check() {
        Ok(Some(rel)) => {
            shared.log(i18n::log_update_available(&rel.version));
            set_state(&shared, UpdateState::Available(rel));
        }
        Ok(None) => set_state(&shared, UpdateState::UpToDate),
        Err(e) => {
            if manual {
                shared.log(i18n::log_update_check_failed(&e));
            }
            set_state(&shared, UpdateState::Error(e));
        }
    });
}

/// Установка в фоне; при успехе процесс завершается, новая версия запускается сама.
pub fn spawn_install(shared: SharedRef, rel: Release) {
    {
        let mut st = shared.update.lock().unwrap();
        if matches!(*st, UpdateState::Installing) {
            return;
        }
        *st = UpdateState::Installing;
    }
    shared.request_repaint();
    std::thread::spawn(move || {
        let sh = shared.clone();
        match install(&rel, &move |s| sh.log(s)) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                shared.log(i18n::log_update_failed(&e));
                set_state(&shared, UpdateState::Error(e));
            }
        }
    });
}

/// Автопроверка: через минуту после запуска, затем раз в 12 часов (если включена).
pub fn spawn_auto_checker(shared: SharedRef, cfg: Arc<Mutex<Config>>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(60));
        loop {
            if cfg.lock().unwrap().check_updates {
                spawn_check(shared.clone(), false);
            }
            std::thread::sleep(Duration::from_secs(12 * 3600));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v2.0.0-beta.1"), Some((2, 0, 0)));
        assert_eq!(parse_version("latest"), None);
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
        assert!(!is_newer(CURRENT_VERSION));
        assert!(is_newer("v999.0.0"));
        assert!(!is_newer("v0.0.1"));
        assert!(!is_newer("garbage"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gp-guard-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn swap_in_replaces_exe_and_keeps_old_copy() {
        let dir = temp_dir("swap");
        let exe = dir.join("gp-guard.exe");
        std::fs::write(&exe, b"MZ old").unwrap();
        std::fs::write(dir.join("gp-guard.exe.old"), b"stale").unwrap();

        swap_in(&exe, b"MZ new").unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ new");
        assert_eq!(std::fs::read(dir.join("gp-guard.exe.old")).unwrap(), b"MZ old");
        assert!(!dir.join("gp-guard.exe.new").exists());

        roll_back(&exe);
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
        assert!(!dir.join("gp-guard.exe.new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swap_in_fails_cleanly_when_exe_is_missing() {
        let dir = temp_dir("missing");
        let exe = dir.join("gp-guard.exe");
        assert!(swap_in(&exe, b"MZ new").is_err());
        assert!(!dir.join("gp-guard.exe.new").exists(), "temporary file must be removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_rejects_foreign_urls() {
        let rel = Release {
            version: "9.9.9".into(),
            page_url: String::new(),
            asset_url: "https://example.com/gp-guard.exe".into(),
            asset_size: 0,
            asset_digest: None,
        };
        assert!(download(&rel).is_err());
    }

    /// Сеть: последний релиз на GitHub скачивается и проходит проверку размера/SHA-256.
    #[test]
    #[ignore = "network"]
    fn live_latest_release_downloads_and_verifies() {
        let rel = fetch_latest().expect("latest release");
        assert!(rel.asset_digest.is_some(), "GitHub should publish a sha256 digest");
        let bytes = download(&rel).expect("download + verify");
        assert_eq!(bytes.len() as u64, rel.asset_size);
    }
}
