//! GP Guard — фоновая оболочка для клиента GlobalProtect:
//! запускает PanGPA.exe, хранит профиль (пароль — в Windows Credential Manager)
//! и переподключает VPN при обрыве.

#![windows_subsystem = "windows"]

mod config;
mod gp;
mod i18n;
mod shared;
mod supervisor;
mod tray;
mod ui;
mod updater;

use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};

const WINDOW_TITLE: &str = "GP Guard";

/// `gp-guard.exe --diag`: записать в журнал состояние GP и элементы его окна и выйти.
fn diag() {
    let shared = shared::Shared::new();
    shared.log(i18n::t(i18n::Msg::LogDiagBanner));
    shared.log(i18n::log_diag_gp_running(gp::is_gp_running()));
    shared.log(i18n::log_diag_tunnel_ip(&gp::vpn_ip()));
    shared.log(i18n::log_diag_gp_window(gp::find_gp_panel().is_some()));
    for l in gp::dump() {
        shared.log(format!("  {l}"));
    }
}

fn main() -> eframe::Result<()> {
    let cfg = config::Config::load();
    // язык журнала и интерфейса: из настроек или системный ("auto")
    i18n::apply_setting(&cfg.language);
    if std::env::args().any(|a| a == "--diag") {
        diag();
        return Ok(());
    }
    let minimized_arg = std::env::args().any(|a| a == "--minimized");
    let after_update = std::env::args().any(|a| a == updater::AFTER_UPDATE_ARG);
    // один экземпляр на пользователя; повторный ручной запуск показывает окно,
    // а повторный запуск из автозагрузки (задача + Run) молча завершается
    unsafe {
        let mutex = CreateMutexW(None, true, w!("Local\\GpGuardSingleInstance"));
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        if exists && after_update {
            // новая версия после обновления: ждём, пока старая завершится и отпустит мьютекс
            if let Ok(h) = &mutex {
                let r = WaitForSingleObject(*h, 30_000);
                if r != WAIT_OBJECT_0 && r != WAIT_ABANDONED {
                    return Ok(());
                }
            }
        } else if exists {
            if minimized_arg {
                return Ok(());
            }
            if let Ok(h) = FindWindowW(None, w!("GP Guard")) {
                let _ = ShowWindow(h, SW_SHOW);
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
            return Ok(());
        }
        std::mem::forget(mutex); // держим мьютекс до завершения процесса
    }

    // после обновления показываем окно, чтобы было видно, что версия сменилась
    let minimized = !after_update && (minimized_arg || cfg.start_minimized);
    let shared = shared::Shared::new();
    if after_update {
        shared.log(i18n::log_updated_to(updater::CURRENT_VERSION));
    }
    updater::cleanup_old_exe();

    // путь к exe мог измениться — актуализируем автозапуск (задача Планировщика
    // запускает schtasks, поэтому в фоне, чтобы не задерживать появление окна)
    if cfg.autostart {
        let sh = shared.clone();
        std::thread::spawn(move || {
            if let Err(e) = config::set_autostart(true) {
                sh.log(i18n::log_autostart_update_failed(&e));
            }
        });
    }
    let first_run = !cfg.is_profile_complete();

    let cfg = Arc::new(Mutex::new(cfg));
    let paused = Arc::new(AtomicBool::new(false));
    let ui_handle = Arc::new(tray::UiHandle::default());
    let (tx, rx) = mpsc::channel();

    supervisor::spawn(shared.clone(), cfg.clone(), rx);
    tray::spawn(shared.clone(), tx.clone(), ui_handle.clone(), paused.clone());
    updater::spawn_auto_checker(shared.clone(), cfg.clone());

    let icon = egui::IconData { rgba: tray::make_icon_rgba([46, 160, 67], 64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size([980.0, 700.0])
            .with_min_inner_size([560.0, 420.0])
            .with_icon(Arc::new(icon))
            // без профиля показываем окно сразу, иначе — только иконка в трее
            .with_visible(first_run || !minimized),
        ..Default::default()
    };

    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, shared, cfg, tx, ui_handle, paused)))),
    )
}
