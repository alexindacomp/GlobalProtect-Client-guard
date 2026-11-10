//! Взаимодействие с установленным клиентом GlobalProtect:
//! процесс PanGPA.exe, состояние туннеля (адаптер PANGP) и
//! управление панелью GP через Win32-сообщения её контролам.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::i18n::{self, t, Msg};

use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetDlgCtrlID, GetParent, GetWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, PostMessageW, SendMessageTimeoutW, SetForegroundWindow, ShowWindow,
    GWL_STYLE, GW_CHILD, GW_HWNDNEXT, SMTO_ABORTIFHUNG, SW_SHOW, WM_COMMAND, WM_SETTEXT,
};

pub const GP_PROCESS: &str = "PanGPA.exe";

// ---------------------------------------------------------------- процессы

pub fn find_pids(exe_name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(exe_name) {
                out.push(entry.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn is_gp_running() -> bool {
    !find_pids(GP_PROCESS).is_empty()
}

pub fn start_gp(path: &str) -> Result<(), String> {
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| i18n::gp_start_failed(path, &e.to_string()))
}

pub fn kill_gp() -> usize {
    let mut killed = 0;
    for pid in find_pids(GP_PROCESS) {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                if TerminateProcess(h, 1).is_ok() {
                    killed += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    killed
}

// ---------------------------------------------------------------- состояние туннеля

/// IPv4-адрес виртуального адаптера GlobalProtect, если он поднят.
pub fn vpn_ip() -> Option<String> {
    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 32 * 1024;
        let mut buf: Vec<u64> = Vec::new(); // u64 — ради выравнивания структуры
        let mut ok = false;
        for _ in 0..3 {
            buf.resize((size as usize).div_ceil(8), 0);
            let ret = GetAdaptersAddresses(
                AF_INET.0 as u32,
                flags,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            );
            if ret == 0 {
                ok = true;
                break;
            }
            if ret != 111 {
                // 111 = ERROR_BUFFER_OVERFLOW, иначе — реальная ошибка
                return None;
            }
        }
        if !ok {
            return None;
        }
        let mut cur = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !cur.is_null() {
            let a = &*cur;
            let desc = a.Description.to_string().unwrap_or_default();
            if desc.to_uppercase().contains("PANGP") && a.OperStatus == IfOperStatusUp {
                let mut ua = a.FirstUnicastAddress;
                while !ua.is_null() {
                    let sa = (*ua).Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*(sa as *const SOCKADDR_IN);
                        let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr));
                        if !ip.is_link_local() {
                            return Some(ip.to_string());
                        }
                    }
                    ua = (*ua).Next;
                }
            }
            cur = a.Next;
        }
        None
    }
}

/// TCP-проверка "host:port" (пустая строка = проверка выключена).
pub fn health_check(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Ok(());
    }
    let addrs: Vec<_> = target.to_socket_addrs().map_err(|e| format!("{target}: {e}"))?.collect();
    let mut last = t(Msg::GpAddressNotResolved).to_string();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("{target}: {last}"))
}

// ---------------------------------------------------------------- панель GP
//
// Панель GP (PanGPA 6.x) — MFC-диалог "#32770" с заголовком "GlobalProtect".
// Каждый "экран" — дочерний диалог; видимый (WS_VISIBLE) — текущий, причём
// это верно и когда сама панель скрыта. Элементы — по стабильным ID контролов:
//   статус:  1160 Connect/Disconnect, 1165 текст статуса
//   вход:    1166 пользователь, 1167 пароль, 1168 Connect, 1170 сообщение
//   портал:  1128 адрес, 1229 Connect
//   прочее:  1329 Get Started, 1531 выбор шлюза, 1464 Refresh Connection,
//            1309 предупреждение о сертификате, 1150 смена пароля

const WS_VISIBLE_BIT: i32 = 0x1000_0000;
const ES_PASSWORD_BIT: i32 = 0x20;

const ID_STATUS_BUTTON: i32 = 1160;
const ID_STATUS_TEXT: i32 = 1165;
const ID_LOGIN_USER: i32 = 1166;
const ID_LOGIN_PASS: i32 = 1167;
const ID_LOGIN_SUBMIT: i32 = 1168;
const ID_LOGIN_MESSAGE: i32 = 1170;
const ID_PORTAL_EDIT: i32 = 1128;
const ID_PORTAL_SUBMIT: i32 = 1229;
const ID_WELCOME_START: i32 = 1329;
const ID_GATEWAY_CONNECT: i32 = 1531;
const ID_REFRESH: i32 = 1464;
const ID_CERT_PROCEED: i32 = 1309;
const ID_PWCHANGE_NEW: i32 = 1150;

fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn style(h: HWND) -> i32 {
    unsafe { GetWindowLongW(h, GWL_STYLE) }
}

fn children(parent: HWND) -> Vec<HWND> {
    let mut out = Vec::new();
    unsafe {
        let mut h = GetWindow(parent, GW_CHILD).unwrap_or_default();
        while !h.is_invalid() {
            out.push(h);
            h = GetWindow(h, GW_HWNDNEXT).unwrap_or_default();
        }
    }
    out
}

#[derive(Clone, Debug)]
struct Ctl {
    hwnd: HWND,
    id: i32,
    class: String,
    text: String,
    visible: bool,
    password: bool,
}

fn ctl(h: HWND) -> Ctl {
    let st = style(h);
    let class = class_name(h);
    Ctl {
        hwnd: h,
        id: unsafe { GetDlgCtrlID(h) },
        password: class == "Edit" && st & ES_PASSWORD_BIT != 0,
        visible: st & WS_VISIBLE_BIT != 0,
        text: window_text(h),
        class,
    }
}

struct Page {
    visible: bool,
    ctls: Vec<Ctl>,
}

impl Page {
    fn get(&self, id: i32) -> Option<&Ctl> {
        self.ctls.iter().find(|c| c.id == id && c.visible)
    }
    fn has(&self, id: i32) -> bool {
        self.get(id).is_some()
    }
}

fn pages(panel: HWND) -> Vec<Page> {
    children(panel)
        .into_iter()
        .filter(|&h| class_name(h) == "#32770")
        .map(|h| Page { visible: style(h) & WS_VISIBLE_BIT != 0, ctls: children(h).into_iter().map(ctl).collect() })
        .collect()
}

/// Главная панель GP: окно процесса PanGPA, в котором есть экраны статуса/входа.
pub fn find_gp_panel() -> Option<HWND> {
    let pids = find_pids(GP_PROCESS);
    if pids.is_empty() {
        return None;
    }
    struct Ctx {
        pids: Vec<u32>,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn enum_top(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if ctx.pids.contains(&pid) && class_name(h) == "#32770" && window_text(h) == "GlobalProtect" {
            ctx.found.push(h);
        }
        true.into()
    }
    let mut ctx = Ctx { pids, found: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_top), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found.into_iter().find(|&h| {
        pages(h).iter().any(|p| p.ctls.iter().any(|c| c.id == ID_STATUS_TEXT || c.id == ID_LOGIN_SUBMIT))
    })
}

/// Текущий экран панели GP.
#[derive(Debug)]
enum Screen {
    Status { button: Option<Ctl>, text: String },
    Login { user: Option<Ctl>, pass: Ctl, submit: Ctl, message: String },
    Portal { edit: Ctl, submit: Ctl },
    Click { ctl: Ctl, what: &'static str },
    NeedsUser(&'static str),
    Unknown,
}

impl Screen {
    fn short(&self) -> String {
        match self {
            Screen::Status { button, text } => {
                i18n::gp_status_screen(text.trim(), button.as_ref().map(|b| b.text.trim()).unwrap_or("-"))
            }
            Screen::Login { message, .. } => i18n::gp_login_screen(message.trim()),
            Screen::Portal { .. } => t(Msg::GpPortalEntry).into(),
            Screen::Click { what, .. } => i18n::gp_click_screen(what),
            Screen::NeedsUser(s) => s.to_string(),
            Screen::Unknown => t(Msg::GpUnknownScreen).into(),
        }
    }
}

fn screen(panel: HWND) -> Screen {
    let all = pages(panel);
    let Some(p) = all.iter().find(|p| p.visible) else { return Screen::Unknown };
    if let (Some(pass), Some(submit)) = (p.get(ID_LOGIN_PASS), p.get(ID_LOGIN_SUBMIT)) {
        return Screen::Login {
            user: p.get(ID_LOGIN_USER).cloned(),
            pass: pass.clone(),
            submit: submit.clone(),
            message: p.get(ID_LOGIN_MESSAGE).map(|c| c.text.clone()).unwrap_or_default(),
        };
    }
    if p.has(ID_STATUS_TEXT) {
        return Screen::Status {
            button: p.get(ID_STATUS_BUTTON).cloned(),
            text: p.get(ID_STATUS_TEXT).map(|c| c.text.clone()).unwrap_or_default(),
        };
    }
    if let (Some(edit), Some(submit)) = (p.get(ID_PORTAL_EDIT), p.get(ID_PORTAL_SUBMIT)) {
        return Screen::Portal { edit: edit.clone(), submit: submit.clone() };
    }
    if let Some(c) = p.get(ID_WELCOME_START) {
        return Screen::Click { ctl: c.clone(), what: "Get Started" };
    }
    if let Some(c) = p.get(ID_GATEWAY_CONNECT) {
        return Screen::Click { ctl: c.clone(), what: t(Msg::GpGatewayConnect) };
    }
    if let Some(c) = p.get(ID_REFRESH) {
        return Screen::Click { ctl: c.clone(), what: "Refresh Connection" };
    }
    if p.has(ID_CERT_PROCEED) {
        return Screen::NeedsUser(t(Msg::GpUntrustedCert));
    }
    if p.has(ID_PWCHANGE_NEW) {
        return Screen::NeedsUser(t(Msg::GpPasswordChange));
    }
    Screen::Unknown
}

/// WM_SETTEXT система корректно передаёт между процессами (и для полей-паролей).
fn set_text(c: &Ctl, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            c.hwnd,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(wide.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            3000,
            Some(&mut result),
        )
    };
    ok.0 != 0 && result != 0
}

/// "Нажатие" кнопки: WM_COMMAND/BN_CLICKED родительскому диалогу —
/// работает и при скрытой панели GP, без фокуса и эмуляции мыши.
fn click(c: &Ctl) -> bool {
    unsafe {
        let Ok(parent) = GetParent(c.hwnd) else { return false };
        if let Ok(panel) = GetParent(parent) {
            if !ensure_panel_open(panel) {
                return false;
            }
        }
        PostMessageW(parent, WM_COMMAND, WPARAM(c.id as u16 as usize), LPARAM(c.hwnd.0 as isize)).is_ok()
    }
}

/// Открыть панель GP так, как это делает пользователь, — кликом по иконке GP в трее
/// (через UI Automation). Важно: GP игнорирует нажатия кнопок, если панель открыта
/// не им самим ("Receive Connect button click event when main panel is invisible").
/// Требует CoInitializeEx на текущем потоке.
fn open_panel_via_tray() -> bool {
    use windows::core::w;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Accessibility::*;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    unsafe fn find_gp_icon(auto: &IUIAutomation, host: HWND) -> Option<IUIAutomationElement> {
        let root = auto.ElementFromHandle(host).ok()?;
        let all = root.FindAll(TreeScope_Descendants, &auto.CreateTrueCondition().ok()?).ok()?;
        for i in 0..all.Length().unwrap_or(0) {
            let Ok(e) = all.GetElement(i) else { continue };
            let name = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let class = e.CurrentClassName().map(|b| b.to_string()).unwrap_or_default();
            if name.starts_with("GlobalProtect") && class.starts_with("SystemTray") {
                return Some(e);
            }
        }
        None
    }
    unsafe fn invoke(e: &IUIAutomationElement) -> bool {
        e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            .and_then(|p| p.Invoke())
            .is_ok()
    }

    unsafe {
        let Ok(auto) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else {
            return false;
        };
        let Ok(taskbar) = FindWindowW(w!("Shell_TrayWnd"), None) else { return false };
        // иконка может быть прямо на панели задач…
        if let Some(icon) = find_gp_icon(&auto, taskbar) {
            return invoke(&icon);
        }
        // …или в области скрытых значков: открываем её и ищем там
        let overflow = || FindWindowW(w!("TopLevelWindowForOverflowXamlIsland"), None).ok();
        let mut icon = overflow().and_then(|h| find_gp_icon(&auto, h));
        if icon.is_none() {
            let root = auto.ElementFromHandle(taskbar).ok();
            let chevron = root.and_then(|r| {
                let cond = auto.CreateTrueCondition().ok()?;
                let all = r.FindAll(TreeScope_Descendants, &cond).ok()?;
                (0..all.Length().unwrap_or(0)).filter_map(|i| all.GetElement(i).ok()).find(|e| {
                    e.CurrentAutomationId().map(|b| "SystemTrayIcon" == b).unwrap_or(false)
                })
            });
            if let Some(ch) = chevron {
                invoke(&ch);
                for _ in 0..10 {
                    std::thread::sleep(Duration::from_millis(200));
                    icon = overflow().and_then(|h| find_gp_icon(&auto, h));
                    if icon.is_some() {
                        break;
                    }
                }
            }
        }
        icon.is_some_and(|i| invoke(&i))
    }
}

fn panel_visible(panel: HWND) -> bool {
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(panel).as_bool() }
}

/// Убедиться, что панель GP открыта "по-настоящему" (через трей), иначе GP не примет клики.
fn ensure_panel_open(panel: HWND) -> bool {
    if panel_visible(panel) {
        return true;
    }
    if !open_panel_via_tray() {
        return false;
    }
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(200));
        if panel_visible(panel) {
            return true;
        }
    }
    false
}

/// Показать панель GP.
pub fn show_gp_panel() -> bool {
    let Some(h) = find_gp_panel() else { return false };
    if !ensure_panel_open(h) {
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = SetForegroundWindow(h);
        }
    }
    true
}

#[derive(Debug)]
pub enum ConnectError {
    /// GP не принял логин/пароль.
    AuthFailed(String),
    /// Нужна реакция пользователя (сертификат, смена пароля и т.п.).
    NeedsUser(&'static str),
    NoWindow,
    Timeout(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::AuthFailed(m) if !m.trim().is_empty() => f.write_str(&i18n::gp_auth_rejected_msg(m.trim())),
            ConnectError::AuthFailed(_) => f.write_str(t(Msg::GpAuthRejected)),
            ConnectError::NeedsUser(s) => f.write_str(&i18n::gp_needs_user(s)),
            ConnectError::NoWindow => f.write_str(t(Msg::GpNoPanel)),
            ConnectError::Timeout(s) => f.write_str(&i18n::gp_timeout(s)),
        }
    }
}

fn is_connect_label(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    s == "connect" || s.starts_with("подключ") || s.starts_with("підключ") || s.starts_with("під'єдн")
}

/// Подключение через панель GP: нажимает Connect, при необходимости вводит
/// портал / логин / пароль. Возвращается, когда туннель поднят, либо с ошибкой.
pub fn connect(
    portal: &str,
    username: &str,
    password: &str,
    timeout: Duration,
    log: &dyn Fn(String),
) -> Result<(), ConnectError> {
    let deadline = Instant::now() + timeout;
    let mut creds_submitted: Option<(Instant, String)> = None;
    let mut last_click: Option<Instant> = None;
    let mut portal_submitted = false;
    let mut last_screen = String::new();
    let mut seen_panel = false;
    let can_click = |t: &Option<Instant>| t.is_none_or(|t| t.elapsed() > Duration::from_secs(20));

    while Instant::now() < deadline {
        if vpn_ip().is_some() {
            return Ok(());
        }
        let Some(panel) = find_gp_panel() else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        seen_panel = true;
        let scr = screen(panel);
        let short = scr.short();
        if short != last_screen {
            log(format!("GP: {short}"));
            last_screen = short;
        }
        match scr {
            Screen::Login { user, pass, submit, message } => {
                if let Some((t, old_msg)) = &creds_submitted {
                    // Форма осталась или вернулась с новым сообщением — вход не принят.
                    // Повторно пароль не вводим, чтобы не заблокировать учётную запись.
                    let changed = message != *old_msg && t.elapsed() > Duration::from_secs(3);
                    if changed || t.elapsed() > Duration::from_secs(20) {
                        return Err(ConnectError::AuthFailed(message));
                    }
                } else {
                    if let Some(u) = &user {
                        if !username.is_empty() {
                            set_text(u, username);
                        }
                    }
                    if !set_text(&pass, password) {
                        log(t(Msg::GpPasswordFillFailed).into());
                    }
                    std::thread::sleep(Duration::from_millis(300));
                    click(&submit);
                    log(t(Msg::GpCredentialsSubmitted).into());
                    creds_submitted = Some((Instant::now(), message));
                }
            }
            Screen::Status { button, .. } => {
                if let Some(b) = button.filter(|b| is_connect_label(&b.text)) {
                    if can_click(&last_click) && creds_submitted.is_none() {
                        log(i18n::gp_pressing(b.text.trim()));
                        click(&b);
                        last_click = Some(Instant::now());
                    }
                }
            }
            Screen::Portal { edit, submit } => {
                if portal.is_empty() {
                    show_gp_panel();
                    return Err(ConnectError::NeedsUser(t(Msg::GpPortalMissing)));
                }
                if !portal_submitted {
                    set_text(&edit, portal);
                    click(&submit);
                    log(i18n::gp_portal_entered(portal));
                    portal_submitted = true;
                }
            }
            Screen::Click { ctl, what } => {
                if can_click(&last_click) {
                    log(i18n::gp_pressing(what));
                    click(&ctl);
                    last_click = Some(Instant::now());
                }
            }
            Screen::NeedsUser(why) => {
                show_gp_panel();
                return Err(ConnectError::NeedsUser(why));
            }
            Screen::Unknown => {}
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if !seen_panel {
        return Err(ConnectError::NoWindow);
    }
    Err(ConnectError::Timeout(i18n::gp_tunnel_not_up(timeout.as_secs())))
}

/// Нажимает "Disconnect" в GP (для "зависшего" туннеля) и ждёт, пока адаптер опустится.
pub fn disconnect(log: &dyn Fn(String)) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut clicked = false;
    while Instant::now() < deadline {
        if vpn_ip().is_none() {
            return true;
        }
        if !clicked {
            if let Some(Screen::Status { button: Some(b), .. }) = find_gp_panel().map(screen) {
                if !is_connect_label(&b.text) {
                    log(i18n::gp_pressing(b.text.trim()));
                    clicked = click(&b);
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    false
}

/// Диагностика: текущий экран и видимые элементы панели GP.
pub fn dump() -> Vec<String> {
    let Some(panel) = find_gp_panel() else { return vec![t(Msg::GpPanelNotFound).into()] };
    let mut out = vec![i18n::gp_dump_panel(panel.0 as usize, &screen(panel).short())];
    for (i, p) in pages(panel).iter().enumerate().filter(|(_, p)| p.visible) {
        out.push(i18n::gp_dump_page(i));
        for c in p.ctls.iter().filter(|c| c.visible) {
            let text = if c.password { t(Msg::GpPasswordPlaceholder).to_string() } else { c.text.chars().take(60).collect() };
            out.push(format!("  id={} {} '{}'", c.id, c.class, text));
        }
    }
    out
}
