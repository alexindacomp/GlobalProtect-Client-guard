//! Иконка в трее. Работает в собственном потоке со своим циклом сообщений,
//! поэтому не зависит от того, скрыто ли окно egui.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::core::HSTRING;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MessageBoxW, MsgWaitForMultipleObjects, PeekMessageW, SetForegroundWindow, ShowWindow,
    TranslateMessage, IDOK, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MB_TOPMOST, MSG, PM_REMOVE,
    QS_ALLINPUT, SW_RESTORE, SW_SHOW,
};

use crate::i18n::{self, t, Msg};
use crate::shared::{restart_confirm_title, Cmd, ConnState, SharedRef};

/// Перезапуск GP из трея: при живом туннеле — только после подтверждения.
fn restart_gp_confirmed(shared: &SharedRef, tx: &Sender<Cmd>) {
    let st = shared.status();
    if st.restart_needs_confirm() {
        let text = HSTRING::from(format!("{}\n\n{}", restart_confirm_title(), st.restart_confirm_text()));
        // модальное окно на потоке трея: MessageBoxW сам прокачивает сообщения
        let answer = unsafe {
            MessageBoxW(
                HWND::default(),
                &text,
                &HSTRING::from("GP Guard"),
                MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST,
            )
        };
        if answer != IDOK {
            return;
        }
    }
    let _ = tx.send(Cmd::RestartGp);
}

/// Данные, которые окно UI передаёт трею после создания.
#[derive(Default)]
pub struct UiHandle {
    pub hwnd: AtomicIsize,
    pub ctx: Mutex<Option<eframe::egui::Context>>,
}

pub fn show_ui(ui: &UiHandle) {
    let h = ui.hwnd.load(Ordering::SeqCst);
    if h != 0 {
        let hwnd = HWND(h as *mut _);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
    if let Some(ctx) = ui.ctx.lock().unwrap().as_ref() {
        ctx.request_repaint();
    }
}

pub fn make_icon_rgba(rgb: [u8; 3], size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let c = (size as f32 - 1.0) / 2.0;
    let r_out = size as f32 / 2.0 - 0.5;
    let r_in = r_out * 0.45;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let i = ((y * size + x) * 4) as usize;
            let alpha = (r_out + 0.5 - d).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            // кольцо цвета статуса с белой "точкой" в центре
            let col = if d < r_in { [255, 255, 255] } else { rgb };
            px[i..i + 4].copy_from_slice(&[col[0], col[1], col[2], (alpha * 255.0) as u8]);
        }
    }
    px
}

fn icon_for(state: ConnState) -> Icon {
    Icon::from_rgba(make_icon_rgba(state.rgb(), 32), 32, 32).expect("valid icon")
}

pub fn spawn(shared: SharedRef, tx: Sender<Cmd>, ui: Arc<UiHandle>, paused: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(shared, tx, ui, paused))
        .expect("spawn tray");
}

fn run(shared: SharedRef, tx: Sender<Cmd>, ui: Arc<UiHandle>, paused: Arc<AtomicBool>) {
    let open = MenuItem::new(t(Msg::TrayOpen), true, None);
    let connect = MenuItem::new(t(Msg::ConnectNow), true, None);
    let restart = MenuItem::new(t(Msg::RestartGp), true, None);
    let show_gp = MenuItem::new(t(Msg::TrayShowGp), true, None);
    let pause = CheckMenuItem::new(t(Msg::PauseMonitoring), true, false, None);
    let quit = MenuItem::new(t(Msg::TrayQuit), true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &open,
        &PredefinedMenuItem::separator(),
        &connect,
        &restart,
        &show_gp,
        &pause,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let mut state = shared.status().state;
    let tray = match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("GP Guard — {}", state.label()))
        .with_icon(icon_for(state))
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            shared.log(i18n::log_tray_icon_failed(&e.to_string()));
            return;
        }
    };

    let mut last_tip = String::new();
    let mut menu_lang = i18n::lang();
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let id = ev.id();
            if id == open.id() {
                show_ui(&ui);
            } else if id == connect.id() {
                let _ = tx.send(Cmd::ConnectNow);
            } else if id == restart.id() {
                restart_gp_confirmed(&shared, &tx);
            } else if id == show_gp.id() {
                let _ = tx.send(Cmd::ShowGp);
            } else if id == pause.id() {
                let p = pause.is_checked();
                paused.store(p, Ordering::SeqCst);
                let _ = tx.send(Cmd::SetPaused(p));
            } else if id == quit.id() {
                shared.log(t(Msg::LogTrayExit));
                drop(tray);
                std::process::exit(0);
            }
        }

        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            match ev {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => show_ui(&ui),
                _ => {}
            }
        }

        // язык мог смениться в окне настроек — обновляем подписи меню
        if i18n::lang() != menu_lang {
            menu_lang = i18n::lang();
            open.set_text(t(Msg::TrayOpen));
            connect.set_text(t(Msg::ConnectNow));
            restart.set_text(t(Msg::RestartGp));
            show_gp.set_text(t(Msg::TrayShowGp));
            pause.set_text(t(Msg::PauseMonitoring));
            quit.set_text(t(Msg::TrayQuit));
        }

        // синхронизация с состоянием (пауза могла быть включена из окна)
        let p = paused.load(Ordering::SeqCst);
        if pause.is_checked() != p {
            pause.set_checked(p);
        }
        let st = shared.status();
        if st.state != state {
            state = st.state;
            let _ = tray.set_icon(Some(icon_for(state)));
        }
        let tip = match &st.vpn_ip {
            Some(ip) if state == ConnState::Connected => format!("GP Guard — {} ({ip})", state.label()),
            _ => format!("GP Guard — {}", state.label()),
        };
        if tip != last_tip {
            let _ = tray.set_tooltip(Some(&tip));
            last_tip = tip;
        }

        unsafe {
            MsgWaitForMultipleObjects(None, false, 300, QS_ALLINPUT);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
