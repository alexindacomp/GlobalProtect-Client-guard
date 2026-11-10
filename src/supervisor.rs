//! Фоновый поток: следит за PanGPA.exe и туннелем, переподключает при обрыве.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{self, Config};
use crate::gp::{self, ConnectError};
use crate::i18n::{self, t, Msg};
use crate::shared::{Cmd, ConnState, SharedRef};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

pub fn spawn(shared: SharedRef, cfg: Arc<Mutex<Config>>, rx: Receiver<Cmd>) {
    std::thread::Builder::new()
        .name("supervisor".into())
        .spawn(move || {
            // UI Automation (открытие панели GP через трей) требует COM на этом потоке
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let mut sv = Supervisor {
                shared,
                cfg,
                paused: false,
                auth_blocked: false,
                ever_connected: false,
                down_since: None,
                unhealthy_since: None,
                last_attempt: None,
                failed_attempts: 0,
            };
            sv.run(rx);
        })
        .expect("spawn supervisor");
}

struct Supervisor {
    shared: SharedRef,
    cfg: Arc<Mutex<Config>>,
    paused: bool,
    /// GP отверг пароль — ждём, пока пользователь поправит профиль.
    auth_blocked: bool,
    ever_connected: bool,
    down_since: Option<Instant>,
    unhealthy_since: Option<Instant>,
    last_attempt: Option<Instant>,
    failed_attempts: u32,
}

impl Supervisor {
    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    fn log(&self, s: impl AsRef<str>) {
        self.shared.log(s);
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        self.log(t(Msg::LogStarted));
        self.tick();
        loop {
            self.shared.record_sample();
            let interval = Duration::from_secs(self.cfg().check_interval_secs.max(2));
            match rx.recv_timeout(interval) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::ConnectNow => {
                self.log(t(Msg::LogManualConnect));
                self.auth_blocked = false;
                if gp::vpn_ip().is_none() {
                    self.attempt(true);
                } else {
                    self.log(t(Msg::LogAlreadyConnected));
                }
            }
            Cmd::RestartGp => {
                self.auth_blocked = false;
                self.restart_gp();
                self.attempt(true);
            }
            Cmd::SetPaused(p) => {
                self.paused = p;
                self.log(t(if p { Msg::LogPaused } else { Msg::LogResumed }));
                if !p {
                    self.down_since = None;
                    self.last_attempt = None;
                }
            }
            Cmd::ConfigChanged => {
                self.auth_blocked = false;
                self.failed_attempts = 0;
                self.last_attempt = None;
                self.log(t(Msg::LogSettingsSaved));
            }
            Cmd::ShowGp => {
                if !gp::is_gp_running() {
                    let _ = gp::start_gp(&self.cfg().gp_path);
                    std::thread::sleep(Duration::from_secs(3));
                }
                if !gp::show_gp_panel() {
                    self.log(t(Msg::LogGpWindowNotFound));
                }
            }
            Cmd::DumpUi => {
                self.log(t(Msg::LogDiagHeader));
                for line in gp::dump() {
                    self.log(format!("  {line}"));
                }
            }
        }
    }

    fn tick(&mut self) {
        let cfg = self.cfg();

        let mut running = gp::is_gp_running();
        if !running && !self.paused {
            self.log(t(Msg::LogStartingGp));
            match gp::start_gp(&cfg.gp_path) {
                Ok(()) => {
                    std::thread::sleep(Duration::from_secs(4));
                    running = gp::is_gp_running();
                }
                Err(e) => self.log(e),
            }
        }

        let ip = gp::vpn_ip();
        let health = if ip.is_some() { gp::health_check(&cfg.health_check) } else { Ok(()) };
        {
            let mut st = self.shared.status.lock().unwrap();
            st.vpn_ip = ip.clone();
            st.gp_running = running;
        }
        let prev = self.shared.status().state;

        if let Some(ip) = &ip {
            match &health {
                Ok(()) => {
                    if prev != ConnState::Connected {
                        self.log(i18n::log_vpn_connected(ip));
                    }
                    self.shared.set_state(ConnState::Connected);
                    self.ever_connected = true;
                    self.down_since = None;
                    self.unhealthy_since = None;
                    self.failed_attempts = 0;
                    self.auth_blocked = false;
                    return;
                }
                Err(e) => {
                    // туннель есть, но внутренняя сеть недоступна — "зависшее" подключение
                    let since = *self.unhealthy_since.get_or_insert_with(|| {
                        self.shared.log(i18n::log_health_failing(e));
                        Instant::now()
                    });
                    self.shared.set_state(ConnState::Disconnected);
                    if self.paused || !cfg.auto_reconnect || since.elapsed() < Duration::from_secs(cfg.grace_secs) {
                        return;
                    }
                    if self.retry_too_soon(&cfg) {
                        return;
                    }
                    self.log(t(Msg::LogReconnectStalled));
                    let sh = self.shared.clone();
                    gp::disconnect(&move |s| sh.log(s));
                    self.unhealthy_since = None;
                    self.attempt(false);
                    return;
                }
            }
        }
        self.unhealthy_since = None;

        if self.paused {
            self.shared.set_state(ConnState::Paused);
            return;
        }
        if !cfg.is_profile_complete() {
            self.shared.set_state(ConnState::NoProfile);
            return;
        }
        if prev == ConnState::Connected {
            self.log(t(Msg::LogVpnDropped));
        }
        self.shared.set_state(ConnState::Disconnected);
        let down = *self.down_since.get_or_insert_with(Instant::now);

        if self.auth_blocked {
            return;
        }
        if self.ever_connected {
            if !cfg.auto_reconnect || down.elapsed() < Duration::from_secs(cfg.grace_secs) {
                return; // даём GP шанс переподключиться самому
            }
        } else if !cfg.connect_on_start {
            return;
        }
        if self.retry_too_soon(&cfg) {
            return;
        }
        self.attempt(false);
    }

    fn retry_too_soon(&self, cfg: &Config) -> bool {
        self.last_attempt
            .is_some_and(|t| t.elapsed() < Duration::from_secs(cfg.retry_interval_secs.max(10)))
    }

    fn restart_gp(&mut self) {
        let cfg = self.cfg();
        let n = gp::kill_gp();
        self.log(i18n::log_restarting_gp(n));
        std::thread::sleep(Duration::from_secs(2));
        if let Err(e) = gp::start_gp(&cfg.gp_path) {
            self.log(e);
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && gp::find_gp_panel().is_none() {
            std::thread::sleep(Duration::from_millis(500));
        }
        std::thread::sleep(Duration::from_secs(2));
        self.shared.status.lock().unwrap().restarts += 1;
    }

    fn attempt(&mut self, manual: bool) {
        let cfg = self.cfg();
        let Some(password) = config::load_password(&cfg.username) else {
            self.log(t(Msg::LogNoPassword));
            self.shared.set_state(ConnState::NoProfile);
            return;
        };
        if !manual && self.failed_attempts >= cfg.attempts_before_restart.max(1) {
            self.restart_gp();
            self.failed_attempts = 0;
        }
        self.last_attempt = Some(Instant::now());
        self.shared.set_state(ConnState::Connecting);
        self.shared.status.lock().unwrap().reconnects += 1;
        self.log(t(Msg::LogConnecting));

        let sh = self.shared.clone();
        let res = gp::connect(&cfg.portal, &cfg.username, &password, CONNECT_TIMEOUT, &move |s| sh.log(s));
        drop(password);
        match res {
            Ok(()) => {
                self.failed_attempts = 0;
                self.ever_connected = true;
                self.down_since = None;
                let ip = gp::vpn_ip();
                self.log(i18n::log_vpn_connected(ip.as_deref().unwrap_or("?")));
                self.shared.status.lock().unwrap().vpn_ip = ip;
                self.shared.set_state(ConnState::Connected);
            }
            Err(e @ (ConnectError::AuthFailed(_) | ConnectError::NeedsUser(_))) => {
                // не повторяем автоматически: неверный пароль может заблокировать учётку,
                // а сертификат/смена пароля требуют решения человека
                self.auth_blocked = true;
                self.log(i18n::log_auto_retry_stopped(&e.to_string()));
                self.shared.set_state(ConnState::Disconnected);
            }
            Err(e) => {
                self.failed_attempts += 1;
                self.log(i18n::log_attempt_failed(self.failed_attempts, &e.to_string()));
                self.shared.set_state(ConnState::Disconnected);
            }
        }
    }
}
