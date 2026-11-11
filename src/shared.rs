//! Общее состояние между потоками (супервизор, трей, UI) и журнал.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::i18n::{t, Msg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnState {
    Starting,
    Connected,
    Disconnected,
    Connecting,
    Paused,
    NoProfile,
}

impl ConnState {
    pub fn label(self) -> &'static str {
        match self {
            ConnState::Starting => t(Msg::StateStarting),
            ConnState::Connected => t(Msg::StateConnected),
            ConnState::Disconnected => t(Msg::StateDisconnected),
            ConnState::Connecting => t(Msg::StateConnecting),
            ConnState::Paused => t(Msg::StatePaused),
            ConnState::NoProfile => t(Msg::StateNoProfile),
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            ConnState::Connected => [46, 160, 67],
            ConnState::Connecting | ConnState::Starting => [230, 150, 20],
            ConnState::Disconnected => [210, 50, 50],
            ConnState::Paused | ConnState::NoProfile => [130, 130, 130],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: ConnState,
    pub vpn_ip: Option<String>,
    pub gp_running: bool,
    pub reconnects: u32,
    pub restarts: u32,
    pub since: chrono::DateTime<chrono::Local>,
}

/// Заголовок подтверждения перезапуска GP (окно и трей).
pub fn restart_confirm_title() -> &'static str {
    t(Msg::RestartConfirmTitle)
}

impl Status {
    /// Перезапуск GP разорвёт живой туннель — нужно подтверждение пользователя.
    pub fn restart_needs_confirm(&self) -> bool {
        self.state == ConnState::Connected || self.vpn_ip.is_some()
    }

    /// Текст подтверждения перезапуска GP.
    pub fn restart_confirm_text(&self) -> String {
        crate::i18n::restart_confirm_text(self.vpn_ip.as_deref().unwrap_or("—"))
    }
}

pub enum Cmd {
    ConnectNow,
    RestartGp,
    SetPaused(bool),
    ConfigChanged,
    ShowGp,
    DumpUi,
}

/// Отметка состояния подключения для графика истории.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub at: chrono::DateTime<chrono::Local>,
    pub state: ConnState,
}

pub struct Shared {
    pub status: Mutex<Status>,
    log: Mutex<VecDeque<String>>,
    /// Счётчик добавленных строк — версия журнала для кеша в UI.
    log_seq: AtomicU64,
    /// Путь к файлу журнала; мьютекс сериализует запись из разных потоков.
    log_file: Mutex<std::path::PathBuf>,
    /// Была ли уже ошибка записи в файл (чтобы сообщить о ней один раз).
    log_file_failed: std::sync::atomic::AtomicBool,
    history: Mutex<VecDeque<Sample>>,
}

pub type SharedRef = Arc<Shared>;

const LOG_LINES: usize = 500;
/// Сколько минут истории подключения хранить.
pub const HISTORY_MINUTES: i64 = 60;
/// Жёсткий предел числа отметок (интервал проверки может быть всего 2 с).
const HISTORY_MAX: usize = 3600;

impl Shared {
    pub fn new() -> SharedRef {
        let path = crate::config::app_dir().join("gp-guard.log");
        // простая ротация: при старте, если файл больше 1 МБ, начинаем заново
        if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
        Arc::new(Shared {
            status: Mutex::new(Status {
                state: ConnState::Starting,
                vpn_ip: None,
                gp_running: false,
                reconnects: 0,
                restarts: 0,
                since: chrono::Local::now(),
            }),
            log: Mutex::new(VecDeque::with_capacity(LOG_LINES)),
            log_seq: AtomicU64::new(0),
            log_file: Mutex::new(path),
            log_file_failed: std::sync::atomic::AtomicBool::new(false),
            history: Mutex::new(VecDeque::new()),
        })
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        let line = format!("{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg.as_ref());
        // файл открывается на каждую запись: строки редкие, а временная недоступность
        // файла (антивирус, индексатор, сон/гибернация) не отключает журнал навсегда
        let written = {
            let path = self.log_file.lock().unwrap();
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&*path)
                .and_then(|mut f| writeln!(f, "{line}"))
        };
        let mut log = self.log.lock().unwrap();
        if let Err(e) = written {
            if !self.log_file_failed.swap(true, Ordering::Relaxed) {
                log.push_back(format!(
                    "{} {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                    crate::i18n::log_file_write_failed(&e.to_string())
                ));
            }
        } else {
            self.log_file_failed.store(false, Ordering::Relaxed);
        }
        while log.len() >= LOG_LINES {
            log.pop_front();
        }
        log.push_back(line);
        // меняется под той же блокировкой, что и буфер, — читатель видит согласованную пару
        self.log_seq.fetch_add(1, Ordering::Relaxed);
    }

    /// Все строки журнала, если он изменился с версии `seen`; вместе с новой версией.
    pub fn log_lines_since(&self, seen: u64) -> Option<(u64, Vec<String>)> {
        let log = self.log.lock().unwrap();
        let seq = self.log_seq.load(Ordering::Relaxed);
        (seq != seen).then(|| (seq, log.iter().cloned().collect()))
    }

    /// Последние `n` строк журнала (от старых к новым).
    pub fn log_tail(&self, n: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter().skip(log.len().saturating_sub(n)).cloned().collect()
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Пульс истории: текущее состояние раз за цикл проверки супервизора.
    pub fn record_sample(&self) {
        let state = self.status().state;
        self.push_sample(Sample { at: chrono::Local::now(), state });
    }

    /// Хранятся последние `HISTORY_MINUTES` минут плюс одна более ранняя отметка,
    /// чтобы график начинался от левого края.
    fn push_sample(&self, sample: Sample) {
        let cutoff = sample.at - chrono::TimeDelta::minutes(HISTORY_MINUTES);
        let mut h = self.history.lock().unwrap();
        h.push_back(sample);
        while h.len() > HISTORY_MAX || (h.len() > 1 && h[1].at <= cutoff) {
            h.pop_front();
        }
    }

    /// Доступ к истории без копирования.
    pub fn with_history<R>(&self, f: impl FnOnce(&VecDeque<Sample>) -> R) -> R {
        f(&self.history.lock().unwrap())
    }

    pub fn set_state(&self, state: ConnState) {
        let changed_at = {
            let mut s = self.status.lock().unwrap();
            if s.state != state {
                s.state = state;
                s.since = chrono::Local::now();
                Some(s.since)
            } else {
                None
            }
        }; // блокировка статуса снята до блокировки истории
        if let Some(at) = changed_at {
            // фиксируем и кратковременные состояния (например, «Подключение…» на время connect)
            self.push_sample(Sample { at, state });
        }
    }
}
