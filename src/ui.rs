//! Окно статуса и настроек (egui): дашборд с боковой навигацией.
//! Закрытие окна прячет его в трей.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, TimeDelta};
use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Label,
    Layout, Margin, Painter, Rect, Response, RichText, Sense, Shape, StrokeKind, TextFormat, Ui, WidgetInfo,
    WidgetType,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

use crate::config::{self, Config};
use crate::i18n::{self, t, Msg, Severity};
use crate::shared::{restart_confirm_title, Cmd, ConnState, Sample, Shared, SharedRef, Status, HISTORY_MINUTES};
use crate::tray::UiHandle;

/// Дизайн-токены и применение темы. Цвета, радиусы, отступы и шрифты окна берутся только отсюда.
mod theme {
    use std::sync::Arc;

    use eframe::egui::{
        self, style::WidgetVisuals, vec2, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke,
        TextStyle, Theme,
    };

    use crate::shared::ConnState;

    // Поверхности
    pub const BG: Color32 = Color32::from_rgb(13, 15, 20);
    pub const SIDEBAR: Color32 = Color32::from_rgb(17, 19, 26);
    pub const SURFACE: Color32 = Color32::from_rgb(22, 25, 33);
    pub const SURFACE_RAISED: Color32 = Color32::from_rgb(31, 35, 45);
    pub const SURFACE_HOVER: Color32 = Color32::from_rgb(41, 46, 58);
    pub const INPUT_BG: Color32 = Color32::from_rgb(15, 17, 23);
    pub const BORDER: Color32 = Color32::from_rgb(40, 45, 57);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(62, 69, 85);
    pub const BORDER_HOVER: Color32 = Color32::from_rgb(88, 96, 116);
    // Текст
    pub const TEXT: Color32 = Color32::from_rgb(230, 232, 238);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(145, 152, 168);
    // Акцент (белый текст на ACCENT и ACCENT_PRESSED — контраст ≥ 4.5:1)
    pub const ACCENT: Color32 = Color32::from_rgb(91, 99, 240);
    pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(76, 84, 226);
    /// Деструктивное действие: заливка — `danger()` (белый текст ≈ 4.9:1), нажатие — темнее (≈ 6.5:1).
    pub const DANGER_PRESSED: Color32 = Color32::from_rgb(178, 40, 40);
    /// Только для линий/ссылок/индикаторов, не как фон под белым текстом.
    pub const ACCENT_HOVER: Color32 = Color32::from_rgb(118, 126, 252);
    pub const ON_ACCENT: Color32 = Color32::WHITE;

    // Геометрия
    pub const RADIUS_XS: u8 = 3;
    pub const RADIUS_SM: u8 = 8;
    pub const RADIUS: u8 = 12;
    pub const RADIUS_PILL: u8 = 14;
    pub const CARD_PAD: i8 = 18;
    pub const PAGE_PAD: i8 = 28;
    pub const GAP: f32 = 14.0;
    pub const FOCUS_RING: f32 = 2.0;
    pub const MODAL_PAD: i8 = 22;
    /// Затемнение фона под модальным диалогом.
    pub const SCRIM: Color32 = Color32::from_black_alpha(150);
    pub const ICON_STROKE: f32 = 1.5;

    // Типографика
    pub const FS_CAPTION: f32 = 11.0;
    pub const FS_SMALL: f32 = 12.5;
    pub const FS_LABEL: f32 = 13.5;
    pub const FS_BODY: f32 = 14.0;
    pub const FS_MONO: f32 = 12.5;
    pub const FS_CARD_TITLE: f32 = 15.5;
    pub const FS_BRAND: f32 = 16.0;
    pub const FS_IP: f32 = 18.0;
    pub const FS_KPI: f32 = 19.0;
    pub const FS_PAGE_TITLE: f32 = 24.0;
    pub const FS_HERO: f32 = 30.0;

    /// Семейство шрифта для заголовков (Segoe UI Semibold, если есть).
    const SEMIBOLD: &str = "semibold";

    /// Цвета состояний — те же, что у иконки в трее (`ConnState::rgb`).
    pub fn state(s: ConnState) -> Color32 {
        let [r, g, b] = s.rgb();
        Color32::from_rgb(r, g, b)
    }
    pub fn success() -> Color32 {
        state(ConnState::Connected)
    }
    pub fn warning() -> Color32 {
        state(ConnState::Connecting)
    }
    pub fn danger() -> Color32 {
        state(ConnState::Disconnected)
    }
    pub fn neutral() -> Color32 {
        state(ConnState::Paused)
    }

    /// Цвет статуса, осветлённый для текста на тёмной поверхности (читаемый контраст).
    pub fn on_dark(c: Color32) -> Color32 {
        c.lerp_to_gamma(TEXT, 0.35)
    }

    /// Полупрозрачная подложка заданного цвета.
    pub fn wash(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
    }

    /// Линия заданной толщины (типизированная обёртка над `Stroke::new`).
    pub fn stroke(width: f32, color: Color32) -> Stroke {
        Stroke::new(width, color)
    }

    pub fn semibold(size: f32) -> FontId {
        FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
    }

    pub fn apply(ctx: &egui::Context) {
        setup_fonts(ctx);
        ctx.set_theme(Theme::Dark);
        ctx.style_mut_of(Theme::Dark, |s| {
            s.visuals = visuals();
            s.text_styles = [
                (TextStyle::Small, FontId::proportional(FS_SMALL)),
                (TextStyle::Body, FontId::proportional(FS_BODY)),
                (TextStyle::Button, FontId::proportional(FS_BODY)),
                (TextStyle::Heading, semibold(FS_PAGE_TITLE)),
                (TextStyle::Monospace, FontId::monospace(FS_MONO)),
            ]
            .into();
            s.spacing.item_spacing = vec2(8.0, 8.0);
            s.spacing.button_padding = vec2(14.0, 6.0);
            s.spacing.interact_size = vec2(40.0, 30.0);
            s.spacing.menu_margin = Margin::same(8);
        });
    }

    pub fn widget(bg: Color32, border: Color32, fg: Color32, expansion: f32) -> WidgetVisuals {
        WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: stroke(1.0, border),
            corner_radius: CornerRadius::same(RADIUS_SM),
            fg_stroke: stroke(1.0, fg),
            expansion,
        }
    }

    fn visuals() -> egui::Visuals {
        let mut v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = stroke(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(RADIUS);
        v.menu_corner_radius = CornerRadius::same(RADIUS_SM);
        let shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(110) };
        v.window_shadow = shadow;
        v.popup_shadow = shadow;
        v.extreme_bg_color = INPUT_BG;
        v.faint_bg_color = SURFACE;
        v.code_bg_color = SURFACE_RAISED;
        v.hyperlink_color = ACCENT_HOVER;
        v.warn_fg_color = on_dark(warning());
        v.error_fg_color = on_dark(danger());
        v.selection.bg_fill = wash(ACCENT, 120);
        v.selection.stroke = stroke(1.0, ACCENT_HOVER);
        v.text_cursor.stroke = stroke(2.0, ACCENT_HOVER);
        v.widgets.noninteractive = widget(SURFACE, BORDER, TEXT, 0.0);
        v.widgets.inactive = widget(SURFACE_RAISED, BORDER_STRONG, TEXT, 0.0);
        v.widgets.hovered = widget(SURFACE_HOVER, BORDER_HOVER, TEXT, 1.0);
        v.widgets.active = widget(SURFACE_HOVER, ACCENT_HOVER, TEXT, 1.0);
        v.widgets.open = widget(SURFACE_RAISED, BORDER_HOVER, TEXT, 0.0);
        v
    }

    fn load_font(fonts: &mut egui::FontDefinitions, name: &str, path: &str) -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    }

    fn setup_fonts(ctx: &egui::Context) {
        // Segoe UI — системный шрифт Windows с кириллицей; Semibold — для заголовков.
        // Если файлов нет, остаются встроенные шрифты egui.
        let mut fonts = egui::FontDefinitions::default();
        let regular = load_font(&mut fonts, "segoe", r"C:\Windows\Fonts\segoeui.ttf");
        let semibold = load_font(&mut fonts, "segoe-semibold", r"C:\Windows\Fonts\seguisb.ttf");

        let mut proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        if regular {
            proportional.insert(0, "segoe".into());
        }
        let mut heading = proportional.clone();
        if semibold {
            heading.insert(0, "segoe-semibold".into());
        }
        fonts.families.insert(FontFamily::Proportional, proportional);
        fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), heading);
        ctx.set_fonts(fonts);
    }
}

/// Ширина окна, ниже которой боковая панель сворачивается до иконок.
const COMPACT_BELOW: f32 = 760.0;
const SIDEBAR_W: f32 = 220.0;
const SIDEBAR_COMPACT_W: f32 = 64.0;
const NAV_ITEM_H: f32 = 38.0;
const ICON_SIZE: f32 = 18.0;
/// Минимальная ширина KPI-карточки до переноса на следующую строку.
const KPI_MIN_W: f32 = 150.0;
const KPI_PAD: i8 = 14;
/// Максимальная ширина полей ввода.
const FIELD_MAX_W: f32 = 480.0;
const TIMELINE_H: f32 = 30.0;
/// Отметка истории «покрывает» не больше этого времени (сон ПК, зависание).
const SAMPLE_MAX_SPAN_MIN: i64 = 5;
const RECENT_EVENTS: usize = 6;
/// Сколько держать сообщение «Сохранено».
const SAVED_MESSAGE_TTL: Duration = Duration::from_secs(4);
const MODAL_W: f32 = 380.0;
/// Ширина выпадающего списка языков.
const LANG_COMBO_W: f32 = 180.0;
/// Ширина карточки истории, при которой заголовок и сводка помещаются в одну строку.
const TIMELINE_HEADER_ONE_ROW_W: f32 = 520.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Page {
    Overview,
    Profile,
    Settings,
    Log,
}

impl Page {
    const ALL: [Page; 4] = [Page::Overview, Page::Profile, Page::Settings, Page::Log];

    fn title(self) -> &'static str {
        match self {
            Page::Overview => t(Msg::PageOverview),
            Page::Profile => t(Msg::PageProfile),
            Page::Settings => t(Msg::PageSettings),
            Page::Log => t(Msg::PageLog),
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => t(Msg::PageOverviewSub),
            Page::Profile => t(Msg::PageProfileSub),
            Page::Settings => t(Msg::PageSettingsSub),
            Page::Log => t(Msg::PageLogSub),
        }
    }

    /// Страница редактирует профиль/настройки (общая форма и кнопка «Сохранить»).
    fn edits_config(self) -> bool {
        matches!(self, Page::Profile | Page::Settings)
    }
}

/// Какие страницы содержат несохранённые изменения.
#[derive(Clone, Copy)]
struct Dirty {
    profile: bool,
    settings: bool,
}

impl Dirty {
    fn any(self) -> bool {
        self.profile || self.settings
    }

    fn page(self, page: Page) -> bool {
        match page {
            Page::Profile => self.profile,
            Page::Settings => self.settings,
            Page::Overview | Page::Log => false,
        }
    }
}

/// Строка журнала, подготовленная для отрисовки и фильтрации.
struct LogLine {
    lower: String,
    job: LayoutJob,
}

/// Разобранный журнал; пересчитывается только при появлении новых строк или смене фильтра.
#[derive(Default)]
struct LogCache {
    seq: u64,
    lines: Vec<LogLine>,
    shown_for: (u64, String),
    shown: Vec<usize>,
}

impl LogCache {
    fn refresh(&mut self, shared: &Shared, filter: &str) {
        if let Some((seq, lines)) = shared.log_lines_since(self.seq) {
            self.seq = seq;
            self.lines = lines.iter().map(|l| LogLine { lower: l.to_lowercase(), job: log_job(l) }).collect();
        }
        let needle = filter.trim().to_lowercase();
        if self.shown_for.0 != self.seq || self.shown_for.1 != needle {
            self.shown = (0..self.lines.len())
                .filter(|&i| needle.is_empty() || self.lines[i].lower.contains(&needle))
                .collect();
            self.shown_for = (self.seq, needle);
        }
    }
}

pub struct App {
    shared: SharedRef,
    cfg: Arc<Mutex<Config>>,
    tx: Sender<Cmd>,
    ui: Arc<UiHandle>,
    paused: Arc<AtomicBool>,
    form: Config,
    password: String,
    password_saved: bool,
    message: Option<(String, bool)>,
    message_at: Option<Instant>,
    page: Page,
    log_filter: String,
    log_cache: LogCache,
    /// Открыт диалог подтверждения перезапуска GP; `true` — нужно поставить фокус на «Отмена».
    confirm_restart: Option<bool>,
    /// Фактическое состояние автозапуска; `None` — идёт проверка (schtasks в фоне).
    autostart_state: Arc<Mutex<Option<config::AutostartState>>>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: SharedRef,
        cfg: Arc<Mutex<Config>>,
        tx: Sender<Cmd>,
        ui: Arc<UiHandle>,
        paused: Arc<AtomicBool>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                ui.hwnd.store(w.hwnd.get(), Ordering::SeqCst);
            }
        }
        *ui.ctx.lock().unwrap() = Some(cc.egui_ctx.clone());

        let form = cfg.lock().unwrap().clone();
        let password_saved = config::load_password(&form.username).is_some();
        // без сохранённого пароля профиль неполон — сразу открываем его
        let page = if password_saved { Page::Overview } else { Page::Profile };
        Self {
            shared,
            cfg,
            tx,
            ui,
            paused,
            form,
            password: String::new(),
            password_saved,
            message: None,
            message_at: None,
            page,
            log_filter: String::new(),
            log_cache: LogCache::default(),
            confirm_restart: None,
            autostart_state: Arc::new(Mutex::new(None)),
        }
        .with_autostart_refresh(&cc.egui_ctx)
    }

    fn with_autostart_refresh(self, ctx: &egui::Context) -> Self {
        self.refresh_autostart(ctx);
        self
    }

    fn ui_ctx(&self) -> egui::Context {
        self.ui.ctx.lock().unwrap().clone().unwrap_or_default()
    }

    /// Перепроверить автозапуск в фоне (запрос к Планировщику занимает заметное время).
    fn refresh_autostart(&self, ctx: &egui::Context) {
        let slot = self.autostart_state.clone();
        *slot.lock().unwrap() = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // даём фоновой актуализации при старте (main.rs) успеть отработать
            std::thread::sleep(std::time::Duration::from_secs(2));
            let state = config::autostart_state();
            *slot.lock().unwrap() = Some(state);
            ctx.request_repaint();
        });
    }

    fn hide(&self) {
        let h = self.ui.hwnd.load(Ordering::SeqCst);
        if h != 0 {
            unsafe {
                let _ = ShowWindow(HWND(h as *mut _), SW_HIDE);
            }
        }
    }

    fn save(&mut self) {
        let old_user = self.cfg.lock().unwrap().username.clone();
        self.form.username = self.form.username.trim().to_string();
        self.form.portal = self.form.portal.trim().to_string();
        self.form.health_check = self.form.health_check.trim().to_string();

        if !self.password.is_empty() {
            if self.form.username.is_empty() {
                self.message = Some((t(Msg::EnterUsername).into(), false));
                return;
            }
            if let Err(e) = config::store_password(&self.form.username, &self.password) {
                self.message = Some((i18n::save_password_failed(&e), false));
                return;
            }
            self.password.clear();
        } else if old_user != self.form.username && !old_user.is_empty() {
            // перенос пароля на новое имя пользователя
            if let Some(p) = config::load_password(&old_user) {
                let _ = config::store_password(&self.form.username, &p);
            }
        }
        if old_user != self.form.username && !old_user.is_empty() {
            config::delete_password(&old_user);
        }

        let autostart = config::set_autostart(self.form.autostart);
        self.refresh_autostart(&self.ui_ctx());
        if let Err(e) = autostart {
            self.message = Some((i18n::autostart_failed(&e), false));
            return;
        }
        if let Err(e) = self.form.save() {
            self.message = Some((i18n::write_settings_failed(&e.to_string()), false));
            return;
        }
        *self.cfg.lock().unwrap() = self.form.clone();
        self.password_saved = config::load_password(&self.form.username).is_some();
        let _ = self.tx.send(Cmd::ConfigChanged);
        self.message = Some((t(Msg::Saved).into(), true));
    }

    /// Несохранённые изменения по страницам: «Профиль» владеет порталом, пользователем,
    /// паролем и путём к PanGPA.exe, «Настройки» — остальными полями.
    fn dirty(&self) -> Dirty {
        let saved = self.cfg.lock().unwrap();
        let profile = !self.password.is_empty()
            || self.form.portal != saved.portal
            || self.form.username != saved.username
            || self.form.gp_path != saved.gp_path;
        let mut rest = self.form.clone();
        rest.portal.clone_from(&saved.portal);
        rest.username.clone_from(&saved.username);
        rest.gp_path.clone_from(&saved.gp_path);
        Dirty { profile, settings: rest != *saved }
    }

    fn revert(&mut self) {
        self.form = self.cfg.lock().unwrap().clone();
        self.password.clear();
        self.message = None;
        // язык мог быть переключён без сохранения — возвращаем сохранённый
        i18n::apply_setting(&self.form.language);
    }

    /// Пользователь изменил поле формы — прежнее сообщение (в т.ч. ошибка) больше не актуально.
    fn form_edited(&mut self) {
        self.message = None;
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::SetPaused(paused));
    }

    // ---------- боковая панель ----------

    fn sidebar(&mut self, ui: &mut Ui, st: &Status, dirty: Dirty, compact: bool) {
        if compact {
            ui.vertical_centered(logo_mark);
        } else {
            ui.horizontal(|ui| {
                logo_mark(ui);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("GP Guard").font(theme::semibold(theme::FS_BRAND)).color(theme::TEXT));
                    ui.label(RichText::new(t(Msg::BrandSubtitle)).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
                });
            });
        }
        ui.add_space(16.0);
        status_pill(ui, st.state, compact);
        ui.add_space(20.0);
        if !compact {
            ui.label(caption(t(Msg::NavCaption)));
        }
        ui.spacing_mut().item_spacing.y = 4.0;
        for page in Page::ALL {
            if nav_item(ui, page, self.page == page, dirty.page(page), compact).clicked() {
                self.page = page;
            }
        }
        if !compact {
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.label(
                    RichText::new(i18n::version(env!("CARGO_PKG_VERSION")))
                        .size(theme::FS_SMALL)
                        .color(theme::TEXT_MUTED),
                );
            });
        }
    }

    // ---------- панель сохранения (Профиль / Настройки) ----------

    fn save_bar(&mut self, ui: &mut Ui, dirty: bool) {
        let (color, text) = match (&self.message, dirty) {
            (Some((msg, false)), _) => (Some(theme::danger()), msg.clone()),
            (_, true) => (Some(theme::warning()), t(Msg::Unsaved).to_string()),
            (Some((msg, true)), false) => (Some(theme::success()), msg.clone()),
            (None, false) => (None, t(Msg::AllSaved).to_string()),
        };
        // кнопки справа, сообщение — в оставшейся ширине (длинная ошибка переносится, а не выталкивает кнопки)
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(ui, t(Msg::Save), dirty).on_disabled_hover_text(t(Msg::NoChanges)).clicked() {
                self.save();
                self.message_at = Some(Instant::now());
            }
            if dirty && ui.button(t(Msg::Revert)).on_hover_text(t(Msg::RevertHint)).clicked() {
                self.revert();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| match color {
                Some(c) => {
                    dot(ui, c, 4.0);
                    ui.add(Label::new(RichText::new(text).color(theme::on_dark(c))).wrap());
                }
                None => {
                    ui.label(RichText::new(text).color(theme::TEXT_MUTED));
                }
            });
        });
    }

    // ---------- Обзор ----------

    fn page_overview(&mut self, ui: &mut Ui, st: &Status) {
        self.hero(ui, st);
        gap(ui);
        self.kpis(ui, st);
        gap(ui);
        self.actions(ui, st);
        gap(ui);
        self.timeline_card(ui);
        gap(ui);
        self.recent_events(ui);
    }

    fn hero(&mut self, ui: &mut Ui, st: &Status) {
        let color = theme::state(st.state);
        let elapsed = fmt_hms(Local::now() - st.since);
        let prefix = match st.state {
            ConnState::Connected => Some(Msg::DurOnline),
            ConnState::Disconnected => Some(Msg::DurOffline),
            ConnState::Connecting | ConnState::Starting => Some(Msg::DurConnecting),
            ConnState::Paused => Some(Msg::DurPaused),
            ConnState::NoProfile => None,
        };
        let duration = prefix.map(|p| i18n::hero_duration(p, &elapsed, &st.since.format("%H:%M:%S").to_string()));
        let accent_w = 4.0;
        let resp = Frame::new()
            .fill(theme::SURFACE)
            .stroke(theme::stroke(1.0, theme::wash(color, 80)))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin {
                left: theme::CARD_PAD + 6,
                right: theme::CARD_PAD,
                top: theme::CARD_PAD,
                bottom: theme::CARD_PAD,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let wide = ui.available_width() >= 460.0;
                ui.horizontal(|ui| {
                    let (dot_rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    glow_dot(ui.painter(), dot_rect.center(), 9.0, color);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(caption(t(Msg::CaptionState)));
                        ui.label(
                            RichText::new(st.state.label())
                                .font(theme::semibold(theme::FS_HERO))
                                .color(theme::on_dark(color)),
                        );
                        if let Some(d) = &duration {
                            ui.label(RichText::new(d).color(theme::TEXT_MUTED));
                        }
                    });
                    if wide {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ip_block(ui, st, Align::Max));
                    }
                });
                if !wide {
                    ui.add_space(4.0);
                    ip_block(ui, st, Align::Min);
                }
                let hint = match st.state {
                    ConnState::NoProfile => {
                        Some((t(Msg::HintNoProfile), t(Msg::OpenProfile)))
                    }
                    ConnState::Paused => {
                        Some((t(Msg::HintPaused), t(Msg::Resume)))
                    }
                    _ => None,
                };
                if let Some((hint, link)) = hint {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(hint).color(theme::TEXT_MUTED));
                        if ui.link(link).clicked() {
                            if st.state == ConnState::NoProfile {
                                self.page = Page::Profile;
                            } else {
                                self.set_paused(false);
                            }
                        }
                    });
                }
            })
            .response;
        // цветная полоса статуса по левому краю карточки
        let r = resp.rect;
        let bar = Rect::from_min_max(r.left_top(), pos2(r.left() + accent_w, r.bottom()));
        let cr = CornerRadius { nw: theme::RADIUS, sw: theme::RADIUS, ne: 0, se: 0 };
        ui.painter().rect_filled(bar, cr, color);
    }

    fn kpis(&self, ui: &mut Ui, st: &Status) {
        let health = self.cfg.lock().unwrap().health_check.clone();
        let health_on = !health.is_empty();
        let items = [
            Kpi {
                label: "PanGPA",
                value: t(if st.gp_running { Msg::KpiRunning } else { Msg::KpiNotRunning }).into(),
                sub: t(Msg::KpiAgent).into(),
                dot: Some(if st.gp_running { theme::success() } else { theme::danger() }),
                sub_is_free_text: false,
            },
            Kpi {
                label: t(Msg::KpiAttempts),
                value: st.reconnects.to_string(),
                sub: t(Msg::KpiSinceStart).into(),
                dot: None,
                sub_is_free_text: false,
            },
            Kpi {
                label: t(Msg::KpiRestarts),
                value: st.restarts.to_string(),
                sub: t(Msg::KpiSinceStart).into(),
                dot: None,
                sub_is_free_text: false,
            },
            Kpi {
                label: t(Msg::KpiHealth),
                value: t(if health_on { Msg::KpiOn } else { Msg::KpiOff }).into(),
                sub: if health_on { health } else { t(Msg::KpiNotSet).to_string() },
                dot: Some(if health_on { theme::success() } else { theme::neutral() }),
                sub_is_free_text: health_on,
            },
        ];

        let avail = ui.available_width();
        let width_for = |n: usize| (avail - theme::GAP * (n - 1) as f32) / n as f32;
        let cols = [4usize, 2].into_iter().find(|&n| width_for(n) >= KPI_MIN_W).unwrap_or(1);
        let w = width_for(cols).floor();
        for (i, row) in items.chunks(cols).enumerate() {
            if i > 0 {
                gap(ui);
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = theme::GAP;
                for k in row {
                    kpi_card(ui, w, k);
                }
            });
        }
    }

    fn actions(&mut self, ui: &mut Ui, st: &Status) {
        let paused = self.paused.load(Ordering::SeqCst);
        card(ui, |ui| {
            card_title(ui, t(Msg::Actions));
            ui.horizontal_wrapped(|ui| {
                // главное действие зависит от состояния
                if st.state == ConnState::NoProfile {
                    if primary_button(ui, t(Msg::FillProfile), true).clicked() {
                        self.page = Page::Profile;
                    }
                } else if paused {
                    if primary_button(ui, t(Msg::ResumeMonitoring), true).clicked() {
                        self.set_paused(false);
                    }
                } else {
                    let busy = match st.state {
                        ConnState::Connected => Some(t(Msg::ConnectBusyConnected)),
                        ConnState::Connecting => Some(t(Msg::AlreadyConnecting)),
                        _ => None,
                    };
                    let resp = primary_button(ui, t(Msg::ConnectNow), busy.is_none())
                        .on_hover_text(t(Msg::ConnectNowHint))
                        .on_disabled_hover_text(busy.unwrap_or_default());
                    if resp.clicked() {
                        let _ = self.tx.send(Cmd::ConnectNow);
                    }
                }
                if ui
                    .button(t(Msg::RestartGp))
                    .on_hover_text(t(Msg::RestartGpHint))
                    .clicked()
                {
                    // живой туннель разорвётся — сначала спрашиваем
                    if st.restart_needs_confirm() {
                        self.confirm_restart = Some(true);
                    } else {
                        let _ = self.tx.send(Cmd::RestartGp);
                    }
                }
                if ui.button(t(Msg::GpWindow)).on_hover_text(t(Msg::GpWindowHint)).clicked() {
                    let _ = self.tx.send(Cmd::ShowGp);
                }
                // «Возобновить» при паузе уже вынесено в главное действие (кроме случая без профиля)
                if !paused {
                    if ui.button(t(Msg::PauseMonitoring)).clicked() {
                        self.set_paused(true);
                    }
                } else if st.state == ConnState::NoProfile && ui.button(t(Msg::ResumeMonitoring)).clicked() {
                    self.set_paused(false);
                }
            });
        });
    }

    fn timeline_card(&self, ui: &mut Ui) {
        let now = Local::now();
        let runs = self.shared.with_history(|h| build_runs(h, now));
        let total: i64 = runs.iter().map(|r| (r.to - r.from).num_seconds()).sum();
        let online: i64 =
            runs.iter().filter(|r| r.state == ConnState::Connected).map(|r| (r.to - r.from).num_seconds()).sum();
        let summary = if total <= 0 {
            t(Msg::NoData).to_string()
        } else {
            i18n::timeline_summary(online * 100 / total, total / 60)
        };
        card(ui, |ui| {
            let title = i18n::timeline_title(HISTORY_MINUTES);
            if ui.available_width() >= TIMELINE_HEADER_ONE_ROW_W {
                ui.horizontal(|ui| {
                    card_title(ui, &title);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(&summary).color(theme::TEXT_MUTED));
                    });
                });
            } else {
                // узкое окно / длинные переводы: сводка под заголовком, а не поверх него
                card_title(ui, &title);
                ui.label(RichText::new(&summary).color(theme::TEXT_MUTED));
            }
            timeline(ui, &runs, now, &summary);
            ui.add_space(2.0);
            legend(ui);
        });
    }

    fn recent_events(&mut self, ui: &mut Ui) {
        let lines = self.shared.log_tail(RECENT_EVENTS);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                card_title(ui, t(Msg::RecentEvents));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.link(t(Msg::FullLog)).clicked() {
                        self.page = Page::Log;
                    }
                });
            });
            if lines.is_empty() {
                ui.label(RichText::new(t(Msg::NoEvents)).color(theme::TEXT_MUTED));
            }
            ui.spacing_mut().item_spacing.y = 6.0;
            for line in lines.iter().rev() {
                let (ts, msg) = split_log_line(line);
                let sev = i18n::severity(msg);
                ui.horizontal(|ui| {
                    dot(ui, sev.dot_color(), 3.0);
                    ui.label(RichText::new(ts.get(11..).unwrap_or(ts)).monospace().color(theme::TEXT_MUTED));
                    ui.add(Label::new(RichText::new(msg).color(sev.text_color())).truncate());
                });
            }
        });
    }

    // ---------- Профиль ----------

    fn page_profile(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let hint = t(if self.password_saved { Msg::PasswordSavedHint } else { Msg::PasswordNotSaved });
        card(ui, |ui| {
            card_title(ui, t(Msg::Account));
            card_subtitle(ui, t(Msg::AccountSub));
            ui.add_space(6.0);
            changed |= field(
                ui,
                t(Msg::Portal),
                Some(t(Msg::PortalHelp)),
                |ui, w| ui.add(text_input(&mut self.form.portal, w)),
            );
            changed |= field(ui, t(Msg::Username), None, |ui, w| ui.add(text_input(&mut self.form.username, w)));
            changed |= field(ui, t(Msg::Password), None, |ui, w| {
                ui.add(text_input(&mut self.password, w).password(true).hint_text(hint))
            });
            // явная отметка: пустое поле пароля иначе выглядит как «не сохранено»
            if self.password_saved {
                status_note(ui, theme::success(), t(Msg::PasswordSavedBadge));
            } else {
                status_note(ui, theme::warning(), t(Msg::PasswordMissingBadge));
            }
        });
        gap(ui);
        card(ui, |ui| {
            card_title(ui, "GlobalProtect");
            ui.add_space(6.0);
            changed |= field(
                ui,
                "PanGPA.exe",
                Some(t(Msg::GpPathHelp)),
                |ui, w| ui.add(text_input(&mut self.form.gp_path, w)),
            );
        });
        if changed {
            self.form_edited();
        }
    }

    // ---------- Настройки ----------

    fn page_settings(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let lang_before = self.form.language.clone();
        card(ui, |ui| {
            card_title(ui, t(Msg::Interface));
            ui.add_space(4.0);
            changed |= setting_row(ui, t(Msg::Language), t(Msg::LanguageDesc), LANG_COMBO_W + 16.0, |ui| {
                language_combo(ui, &mut self.form.language)
            });
        });
        if self.form.language != lang_before {
            // применяется сразу (окно и трей); в config.json попадает по «Сохранить»
            i18n::apply_setting(&self.form.language);
            ui.ctx().request_repaint();
        }
        gap(ui);
        card(ui, |ui| {
            card_title(ui, t(Msg::Behaviour));
            ui.add_space(4.0);
            changed |= toggle_row(
                ui,
                &mut self.form.autostart,
                t(Msg::Autostart),
                t(Msg::AutostartDesc),
            );
            autostart_status(ui, *self.autostart_state.lock().unwrap());
            ui.separator();
            changed |= toggle_row(
                ui,
                &mut self.form.auto_reconnect,
                t(Msg::AutoReconnect),
                t(Msg::AutoReconnectDesc),
            );
            ui.separator();
            changed |= toggle_row(
                ui,
                &mut self.form.connect_on_start,
                t(Msg::ConnectOnStart),
                t(Msg::ConnectOnStartDesc),
            );
            ui.separator();
            changed |= toggle_row(
                ui,
                &mut self.form.start_minimized,
                t(Msg::StartMinimized),
                t(Msg::StartMinimizedDesc),
            );
        });
        gap(ui);
        card(ui, |ui| {
            card_title(ui, t(Msg::Advanced));
            card_subtitle(ui, t(Msg::AdvancedSub));
            ui.add_space(4.0);
            let f = &mut self.form;
            changed |= setting_row(ui, t(Msg::CheckInterval), t(Msg::CheckIntervalDesc), 110.0, |ui| {
                ui.add(egui::DragValue::new(&mut f.check_interval_secs).range(2..=300).suffix(t(Msg::SecondsSuffix)))
            });
            ui.separator();
            changed |= setting_row(
                ui,
                t(Msg::Grace),
                t(Msg::GraceDesc),
                110.0,
                |ui| ui.add(egui::DragValue::new(&mut f.grace_secs).range(0..=600).suffix(t(Msg::SecondsSuffix))),
            );
            ui.separator();
            changed |= setting_row(
                ui,
                t(Msg::RetryInterval),
                t(Msg::RetryIntervalDesc),
                110.0,
                |ui| ui.add(egui::DragValue::new(&mut f.retry_interval_secs).range(10..=3600).suffix(t(Msg::SecondsSuffix))),
            );
            ui.separator();
            changed |= setting_row(
                ui,
                t(Msg::RestartAfter),
                t(Msg::RestartAfterDesc),
                110.0,
                |ui| ui.add(egui::DragValue::new(&mut f.attempts_before_restart).range(1..=20)),
            );
            ui.separator();
            ui.add_space(4.0);
            changed |= field(
                ui,
                t(Msg::HealthCheck),
                Some(t(Msg::HealthCheckHelp)),
                |ui, w| {
                    ui.add(
                        text_input(&mut f.health_check, w).hint_text(t(Msg::HealthCheckHint)),
                    )
                },
            );
        });
        gap(ui);
        card(ui, |ui| {
            card_title(ui, t(Msg::Diagnostics));
            card_subtitle(
                ui,
                t(Msg::DiagnosticsSub),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(t(Msg::DiagnoseGpWindow)).clicked() {
                    let _ = self.tx.send(Cmd::DumpUi);
                }
                if ui.link(t(Msg::OpenLog)).clicked() {
                    self.page = Page::Log;
                }
            });
        });
        if changed {
            self.form_edited();
        }
    }

    // ---------- подтверждение перезапуска GP ----------

    /// Модальный диалог: Esc, клик по фону и «Отмена» закрывают без действия,
    /// «Перезапустить» отправляет `Cmd::RestartGp`. Фокус по умолчанию — на «Отмена».
    fn restart_modal(&mut self, ctx: &egui::Context, st: &Status, focus_cancel: bool) {
        let resp = egui::Modal::new(egui::Id::new("confirm_restart_gp"))
            .backdrop_color(theme::SCRIM)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(theme::stroke(1.0, theme::BORDER_STRONG))
                    .corner_radius(CornerRadius::same(theme::RADIUS))
                    .inner_margin(Margin::same(theme::MODAL_PAD)),
            )
            .show(ctx, |ui| {
                ui.set_width(MODAL_W);
                ui.label(RichText::new(restart_confirm_title()).font(theme::semibold(theme::FS_CARD_TITLE)).color(theme::TEXT));
                ui.add_space(2.0);
                ui.label(RichText::new(st.restart_confirm_text()).color(theme::TEXT_MUTED));
                ui.add_space(theme::GAP);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // справа налево: «Отмена» крайняя справа, как OK/Cancel в Windows
                    let cancel = ui.button(t(Msg::Cancel));
                    let confirm = danger_button(ui, t(Msg::Restart));
                    if focus_cancel {
                        cancel.request_focus();
                    }
                    (confirm.clicked(), cancel.clicked())
                })
                .inner
            });
        let (confirm, cancel) = resp.inner;
        if confirm {
            let _ = self.tx.send(Cmd::RestartGp);
            self.confirm_restart = None;
        } else if cancel || resp.should_close() {
            self.confirm_restart = None;
        } else {
            self.confirm_restart = Some(false);
        }
    }

    // ---------- Журнал ----------

    fn page_log(&mut self, ui: &mut Ui) {
        page_header(ui, Page::Log);
        self.log_cache.refresh(&self.shared, &self.log_filter);
        let total = self.log_cache.lines.len();
        let shown = self.log_cache.shown.len();

        ui.horizontal(|ui| {
            let w = (ui.available_width() - 230.0).clamp(160.0, 360.0);
            ui.add(text_input(&mut self.log_filter, w).hint_text(t(Msg::FilterHint)));
            if !self.log_filter.is_empty() && ui.button(t(Msg::ClearFilter)).clicked() {
                self.log_filter.clear();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(i18n::log_count(shown, total)).color(theme::TEXT_MUTED));
            });
        });
        gap(ui);
        let filter = self.log_filter.trim();
        let cache = &self.log_cache;
        card(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("log")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if total == 0 {
                        ui.label(RichText::new(t(Msg::LogEmpty)).color(theme::TEXT_MUTED));
                    } else if shown == 0 {
                        ui.label(RichText::new(i18n::log_no_match(filter)).color(theme::TEXT_MUTED));
                    }
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for &i in &cache.shown {
                        ui.add(Label::new(cache.lines[i].job.clone()).wrap());
                    }
                });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_secs(1));

        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide();
            // не показывать устаревший диалог при следующем открытии окна
            self.confirm_restart = None;
            // несохранённая смена языка не переживает сворачивание в трей:
            // возвращаем сохранённый язык (остальные несохранённые правки формы не трогаем)
            let saved_lang = self.cfg.lock().unwrap().language.clone();
            if self.form.language != saved_lang {
                i18n::apply_setting(&saved_lang);
                self.form.language = saved_lang;
            }
        }

        // «Сохранено» гаснет само; ошибки остаются до правки формы
        if matches!(self.message, Some((_, true))) && self.message_at.is_some_and(|t| t.elapsed() >= SAVED_MESSAGE_TTL)
        {
            self.message = None;
        }

        let st = self.shared.status();
        let dirty = self.dirty();
        let compact = ctx.screen_rect().width() < COMPACT_BELOW;

        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(if compact { SIDEBAR_COMPACT_W } else { SIDEBAR_W })
            .frame(
                Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(Margin::symmetric(if compact { 10 } else { 14 }, 18)),
            )
            .show(ctx, |ui| self.sidebar(ui, &st, dirty, compact));

        if self.page.edits_config() {
            egui::TopBottomPanel::bottom("save_bar")
                .frame(Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::symmetric(theme::PAGE_PAD, 12)))
                .show(ctx, |ui| self.save_bar(ui, dirty.any()));
        }

        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG)).show(ctx, |ui| {
            if self.page == Page::Log {
                // журнал прокручивается сам и занимает всю высоту
                Frame::new().inner_margin(Margin::same(theme::PAGE_PAD)).show(ui, |ui| self.page_log(ui));
                return;
            }
            egui::ScrollArea::vertical().id_salt(self.page).auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin::same(theme::PAGE_PAD)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    page_header(ui, self.page);
                    match self.page {
                        Page::Overview => self.page_overview(ui, &st),
                        Page::Profile => self.page_profile(ui),
                        Page::Settings => self.page_settings(ui),
                        Page::Log => {}
                    }
                });
            });
        });

        // туннель уже не поднят — подтверждать нечего
        if !st.restart_needs_confirm() {
            self.confirm_restart = None;
        }
        if let Some(focus_cancel) = self.confirm_restart {
            self.restart_modal(ctx, &st, focus_cancel);
        }
    }
}

// ---------- строительные блоки ----------

fn gap(ui: &mut Ui) {
    let extra = theme::GAP - ui.spacing().item_spacing.y;
    ui.add_space(extra.max(0.0));
}

fn caption(text: &str) -> RichText {
    RichText::new(text).size(theme::FS_CAPTION).color(theme::TEXT_MUTED).extra_letter_spacing(0.8)
}

fn page_header(ui: &mut Ui, page: Page) {
    ui.heading(page.title());
    ui.label(RichText::new(page.subtitle()).color(theme::TEXT_MUTED));
    gap(ui);
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(theme::stroke(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(theme::CARD_PAD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn card_title(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).font(theme::semibold(theme::FS_CARD_TITLE)).color(theme::TEXT));
}

fn card_subtitle(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
}

fn focus_ring(ui: &Ui, rect: Rect, radius: u8) {
    ui.painter().rect_stroke(
        rect.expand(theme::FOCUS_RING),
        CornerRadius::same(radius.saturating_add(theme::FOCUS_RING as u8)),
        theme::stroke(theme::FOCUS_RING, theme::TEXT),
        StrokeKind::Outside,
    );
}

/// Главная кнопка: акцентная заливка, при наведении/нажатии — темнее, видимое кольцо фокуса.
fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    solid_button(ui, text, enabled, theme::ACCENT, theme::ACCENT_PRESSED, theme::ACCENT_HOVER)
}

/// Кнопка подтверждения деструктивного действия (цвет «Нет подключения»).
fn danger_button(ui: &mut Ui, text: &str) -> Response {
    solid_button(ui, text, true, theme::danger(), theme::DANGER_PRESSED, theme::on_dark(theme::danger()))
}

/// Залитая кнопка с белым текстом: при наведении/нажатии — `pressed`, видимое кольцо фокуса.
fn solid_button(ui: &mut Ui, text: &str, enabled: bool, fill: Color32, pressed: Color32, hover_border: Color32) -> Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive = theme::widget(fill, fill, theme::ON_ACCENT, 0.0);
        w.hovered = theme::widget(pressed, hover_border, theme::ON_ACCENT, 1.0);
        w.active = theme::widget(pressed, pressed, theme::ON_ACCENT, 1.0);
        let resp =
            ui.add_enabled(enabled, egui::Button::new(RichText::new(text).font(theme::semibold(theme::FS_BODY))));
        if resp.has_focus() {
            focus_ring(ui, resp.rect, theme::RADIUS_SM);
        }
        resp
    })
    .inner
}

fn text_input(value: &mut String, width: f32) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(value).desired_width(width).margin(Margin::symmetric(10, 6))
}

/// Подпись над полем ввода + необязательная подсказка под ним. Возвращает «значение изменено».
fn field(ui: &mut Ui, label: &str, help: Option<&str>, add: impl FnOnce(&mut Ui, f32) -> Response) -> bool {
    let changed = ui
        .vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let label_id =
                ui.label(RichText::new(label).font(theme::semibold(theme::FS_LABEL)).color(theme::TEXT)).id;
            let w = ui.available_width().min(FIELD_MAX_W);
            let changed = add(ui, w).labelled_by(label_id).changed();
            if let Some(help) = help {
                ui.label(RichText::new(help).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
            }
            changed
        })
        .inner;
    ui.add_space(6.0);
    changed
}

/// Строка настройки: заголовок и описание слева, элемент управления справа.
fn setting_row(
    ui: &mut Ui,
    title: &str,
    desc: &str,
    control_w: f32,
    add: impl FnOnce(&mut Ui) -> Response,
) -> bool {
    ui.horizontal(|ui| {
        let text_w = (ui.available_width() - control_w - theme::GAP).max(120.0);
        let label_id = ui
            .vertical(|ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                let id = ui.label(RichText::new(title).color(theme::TEXT)).id;
                ui.label(RichText::new(desc).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
                id
            })
            .inner;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| add(ui).labelled_by(label_id).changed()).inner
    })
    .inner
}

/// Название языка для списка: «Авто» переводится, остальные — на своём языке.
fn language_label(code: &str) -> &'static str {
    i18n::LANGUAGE_OPTIONS
        .iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, name)| *name)
        .unwrap_or_else(|| t(Msg::LanguageAuto))
}

fn language_combo(ui: &mut Ui, value: &mut String) -> Response {
    let before = value.clone();
    let mut resp = egui::ComboBox::from_id_salt("language")
        .width(LANG_COMBO_W)
        .selected_text(language_label(value))
        .show_ui(ui, |ui| {
            for (code, _) in i18n::LANGUAGE_OPTIONS {
                ui.selectable_value(value, code.to_string(), language_label(code));
            }
        })
        .response;
    if *value != before {
        resp.mark_changed();
    }
    resp
}

fn toggle_row(ui: &mut Ui, value: &mut bool, title: &str, desc: &str) -> bool {
    setting_row(ui, title, desc, 44.0, |ui| toggle(ui, value, title))
}

/// Переключатель в стиле iOS/Linear; доступен с клавиатуры (Tab, Пробел/Enter).
fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let size = vec2(40.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let value = *on;
    let enabled = ui.is_enabled();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, value, label));
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(resp.id, value);
        let radius = rect.height() / 2.0;
        let track = theme::SURFACE_HOVER.lerp_to_gamma(theme::ACCENT, t);
        let border = if resp.hovered() { theme::BORDER_HOVER } else { theme::BORDER_STRONG };
        let p = ui.painter();
        p.rect(rect, CornerRadius::same(radius as u8), track, theme::stroke(1.0, border), StrokeKind::Inside);
        let cx = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        p.circle_filled(pos2(cx, rect.center().y), radius - 4.0, theme::ON_ACCENT);
        if resp.has_focus() {
            focus_ring(ui, rect, radius as u8);
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Небольшая строка состояния: цветная точка и пояснение.
fn status_note(ui: &mut Ui, color: Color32, text: &str) {
    ui.horizontal_wrapped(|ui| {
        dot(ui, color, 3.5);
        ui.label(RichText::new(text).size(theme::FS_SMALL).color(theme::on_dark(color)));
    });
}

/// Фактическое состояние автозапуска под переключателем.
fn autostart_status(ui: &mut Ui, state: Option<config::AutostartState>) {
    ui.add_space(2.0);
    let Some(s) = state else {
        status_note(ui, theme::neutral(), t(Msg::AutostartChecking));
        return;
    };
    let mark = |ok: bool| if ok { "✓" } else { "✗" };
    let detail = format!(
        "{}  ·  {} {}  ·  {} {}",
        t(if s.any() { Msg::AutostartActive } else { Msg::AutostartInactive }),
        t(Msg::AutostartTask),
        mark(s.task),
        t(Msg::AutostartRegistry),
        mark(s.registry),
    );
    let color = match (s.task, s.registry) {
        (true, true) => theme::success(),
        (false, false) => theme::neutral(),
        _ => theme::warning(),
    };
    status_note(ui, color, &detail);
}

fn dot(ui: &mut Ui, color: Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(radius * 2.0 + 2.0, radius * 2.0 + 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
}

/// Точка статуса с мягким свечением.
fn glow_dot(painter: &Painter, center: egui::Pos2, radius: f32, color: Color32) {
    painter.circle_filled(center, radius * 2.4, theme::wash(color, 22));
    painter.circle_filled(center, radius * 1.6, theme::wash(color, 48));
    painter.circle_filled(center, radius, color);
}

fn logo_mark(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::ACCENT);
    p.text(rect.center(), Align2::CENTER_CENTER, "GP", theme::semibold(theme::FS_LABEL), theme::ON_ACCENT);
}

fn status_pill(ui: &mut Ui, state: ConnState, compact: bool) {
    let color = theme::state(state);
    if compact {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
        glow_dot(ui.painter(), rect.center(), 5.0, color);
        resp.on_hover_text(state.label());
        return;
    }
    // плашка по размеру содержимого — не похожа на кнопку
    Frame::new()
        .fill(theme::wash(color, 20))
        .stroke(theme::stroke(1.0, theme::wash(color, 90)))
        .corner_radius(CornerRadius::same(theme::RADIUS_PILL))
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                dot(ui, color, 4.0);
                // длинные переводы («Моніторинг призупинено») переносятся, а не вылезают за панель
                ui.add(
                    Label::new(
                        RichText::new(state.label()).font(theme::semibold(theme::FS_LABEL)).color(theme::on_dark(color)),
                    )
                    .wrap(),
                );
            });
        });
}

/// Векторные иконки навигации (контур 1.5 px в квадрате 18×18).
fn paint_icon(p: &Painter, page: Page, rect: Rect, color: Color32) {
    let s = theme::stroke(theme::ICON_STROKE, color);
    let at = |x: f32, y: f32| rect.min + vec2(x, y);
    match page {
        Page::Overview => {
            for (x, top) in [(2.5, 9.0), (7.5, 3.0), (12.5, 6.0)] {
                let bar = Rect::from_min_max(at(x, top), at(x + 3.0, 15.5));
                p.rect_stroke(bar, CornerRadius::same(1), s, StrokeKind::Middle);
            }
        }
        Page::Profile => {
            p.circle_stroke(at(9.0, 5.5), 3.25, s);
            let shoulders: Vec<_> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                    at(9.0 + 6.0 * a.cos(), 16.0 + 5.0 * a.sin())
                })
                .collect();
            p.add(Shape::line(shoulders, s));
        }
        Page::Settings => {
            for (y, knob) in [(4.0, 6.0), (9.0, 12.0), (14.0, 8.0)] {
                p.line_segment([at(2.0, y), at(16.0, y)], s);
                p.circle_filled(at(knob, y), 2.25, color);
            }
        }
        Page::Log => {
            for y in [4.0, 9.0, 14.0] {
                p.line_segment([at(2.0, y), at(4.0, y)], s);
                p.line_segment([at(6.5, y), at(16.0, y)], s);
            }
        }
    }
}

fn nav_item(ui: &mut Ui, page: Page, selected: bool, unsaved: bool, compact: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), NAV_ITEM_H), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, page.title()));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered();
        if selected {
            p.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::SURFACE_RAISED);
            let bar = Rect::from_center_size(pos2(rect.left() + 1.5, rect.center().y), vec2(3.0, 18.0));
            p.rect_filled(bar, CornerRadius::same(2), theme::ACCENT_HOVER);
        } else if hovered {
            p.rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), theme::wash(theme::SURFACE_RAISED, 150));
        }
        let fg = if selected || hovered { theme::TEXT } else { theme::TEXT_MUTED };
        let cy = rect.center().y;
        let icon_center = if compact { rect.center() } else { pos2(rect.left() + 14.0 + ICON_SIZE / 2.0, cy) };
        paint_icon(p, page, Rect::from_center_size(icon_center, vec2(ICON_SIZE, ICON_SIZE)), fg);
        if compact {
            if unsaved {
                p.circle_filled(pos2(rect.right() - 9.0, rect.top() + 9.0), 3.5, theme::warning());
            }
        } else {
            let font = if selected { theme::semibold(theme::FS_BODY) } else { FontId::proportional(theme::FS_BODY) };
            p.text(pos2(rect.left() + 42.0, cy), Align2::LEFT_CENTER, page.title(), font, fg);
            if unsaved {
                p.circle_filled(pos2(rect.right() - 14.0, cy), 3.5, theme::warning());
            }
        }
        if resp.has_focus() {
            p.rect_stroke(
                rect,
                CornerRadius::same(theme::RADIUS_SM),
                theme::stroke(theme::FOCUS_RING, theme::ACCENT_HOVER),
                StrokeKind::Inside,
            );
        }
    }
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    match (compact, unsaved) {
        (true, true) => resp.on_hover_text(i18n::nav_unsaved(page.title())),
        (true, false) => resp.on_hover_text(page.title()),
        (false, true) => resp.on_hover_text(t(Msg::Unsaved)),
        (false, false) => resp,
    }
}

fn ip_block(ui: &mut Ui, st: &Status, align: Align) {
    ui.with_layout(Layout::top_down(align), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(caption(t(Msg::CaptionTunnelIp)));
        let font = FontId::monospace(theme::FS_IP);
        match &st.vpn_ip {
            Some(ip) => ui.label(RichText::new(ip).font(font).color(theme::TEXT)),
            None => ui.label(RichText::new("—").font(font).color(theme::TEXT_MUTED)),
        };
    });
}

struct Kpi {
    label: &'static str,
    value: String,
    sub: String,
    dot: Option<Color32>,
    /// Подпись — произвольный текст пользователя (адрес): обрезаем с подсказкой.
    sub_is_free_text: bool,
}

fn kpi_card(ui: &mut Ui, width: f32, k: &Kpi) {
    let inner_w = width - 2.0 * (KPI_PAD as f32 + 1.0);
    Frame::new()
        .fill(theme::SURFACE)
        .stroke(theme::stroke(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(KPI_PAD))
        .show(ui, |ui| {
            ui.set_width(inner_w);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(k.label).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if let Some(c) = k.dot {
                        dot(ui, c, 4.0);
                    }
                    ui.label(RichText::new(&k.value).font(theme::semibold(theme::FS_KPI)).color(theme::TEXT));
                });
                let sub = RichText::new(&k.sub).size(theme::FS_SMALL).color(theme::TEXT_MUTED);
                if k.sub_is_free_text {
                    ui.add(Label::new(sub).truncate()).on_hover_text(&k.sub);
                } else {
                    ui.label(sub);
                }
            });
        });
}

// ---------- история подключения ----------

/// Непрерывный отрезок одного состояния на шкале времени.
struct Run {
    from: DateTime<Local>,
    to: DateTime<Local>,
    state: ConnState,
}

/// Отметки состояния → отрезки за последние `HISTORY_MINUTES` минут.
/// Каждая отметка действует до следующей (но не дольше `SAMPLE_MAX_SPAN_MIN`).
fn build_runs(history: &VecDeque<Sample>, now: DateTime<Local>) -> Vec<Run> {
    let start = now - TimeDelta::minutes(HISTORY_MINUTES);
    let max_span = TimeDelta::minutes(SAMPLE_MAX_SPAN_MIN);
    let mut runs: Vec<Run> = Vec::new();
    for (i, s) in history.iter().enumerate() {
        let next = history.get(i + 1).map_or(now, |n| n.at);
        let to = next.min(s.at + max_span).min(now);
        let from = s.at.max(start);
        if to <= from {
            continue;
        }
        match runs.last_mut() {
            Some(r) if r.state == s.state && r.to == from => r.to = to,
            _ => runs.push(Run { from, to, state: s.state }),
        }
    }
    runs
}

fn timeline(ui: &mut Ui, runs: &[Run], now: DateTime<Local>, summary: &str) {
    let start = now - TimeDelta::minutes(HISTORY_MINUTES);
    let span_secs = (HISTORY_MINUTES * 60) as f32;
    let width = ui.available_width();
    let (band, resp) = ui.allocate_exact_size(vec2(width, TIMELINE_H), Sense::hover());
    let (axis, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, i18n::timeline_a11y(summary)));

    let x_at = |t: DateTime<Local>| {
        let secs = (t - start).num_milliseconds() as f32 / 1000.0;
        band.left() + (secs / span_secs).clamp(0.0, 1.0) * band.width()
    };
    let p = ui.painter();
    p.rect_filled(band, CornerRadius::same(theme::RADIUS_SM), theme::SURFACE_RAISED);
    for r in runs {
        let x0 = x_at(r.from);
        let x1 = x_at(r.to).max(x0 + 1.0);
        let left = if x0 <= band.left() + 0.5 { theme::RADIUS_SM } else { 0 };
        let right = if x1 >= band.right() - 0.5 { theme::RADIUS_SM } else { 0 };
        let cr = CornerRadius { nw: left, sw: left, ne: right, se: right };
        p.rect_filled(Rect::from_x_y_ranges(x0..=x1, band.y_range()), cr, theme::state(r.state));
    }
    // начало данных, если история ещё не покрывает весь час (недавний запуск)
    if let Some(first) = runs.first().filter(|r| r.from - start > TimeDelta::minutes(1)) {
        let x = x_at(first.from);
        p.line_segment([pos2(x, band.top()), pos2(x, band.bottom())], theme::stroke(1.0, theme::TEXT_MUTED));
        if x - band.left() >= 160.0 {
            p.text(
                pos2((band.left() + x) / 2.0, band.center().y),
                Align2::CENTER_CENTER,
                i18n::data_since(&first.from.format("%H:%M").to_string()),
                FontId::proportional(theme::FS_CAPTION),
                theme::TEXT_MUTED,
            );
        }
    }
    // деления каждые 15 минут
    let labels = [
        i18n::axis_minutes_ago(60, true),
        i18n::axis_minutes_ago(45, false),
        i18n::axis_minutes_ago(30, false),
        i18n::axis_minutes_ago(15, false),
        t(Msg::Now).to_string(),
    ];
    let font = FontId::proportional(theme::FS_CAPTION);
    for (i, text) in labels.iter().enumerate() {
        let x = band.left() + band.width() * i as f32 / 4.0;
        if i > 0 && i < 4 {
            p.line_segment(
                [pos2(x, band.top()), pos2(x, band.bottom())],
                theme::stroke(1.0, theme::wash(theme::BG, 150)),
            );
        }
        let anchor = match i {
            0 => Align2::LEFT_TOP,
            4 => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        p.text(pos2(x, axis.top() + 2.0), anchor, text, font.clone(), theme::TEXT_MUTED);
    }

    if let Some(pos) = resp.hover_pos() {
        let x = pos.x.clamp(band.left(), band.right());
        p.line_segment([pos2(x, band.top() - 2.0), pos2(x, band.bottom() + 2.0)], theme::stroke(1.5, theme::TEXT));
        let secs = (x - band.left()) / band.width() * span_secs;
        let at = start + TimeDelta::milliseconds((secs * 1000.0) as i64);
        let state = runs.iter().find(|r| r.from <= at && at <= r.to).map(|r| r.state);
        let label = state.map_or(t(Msg::NoData), ConnState::label);
        resp.on_hover_text_at_pointer(format!("{} — {label}", at.format("%H:%M:%S")));
    }
}

fn legend(ui: &mut Ui) {
    let items = [
        (theme::state(ConnState::Connected), t(Msg::LegendConnected)),
        (theme::state(ConnState::Connecting), t(Msg::LegendConnecting)),
        (theme::state(ConnState::Disconnected), t(Msg::LegendOffline)),
        (theme::neutral(), t(Msg::LegendPaused)),
        (theme::SURFACE_RAISED, t(Msg::NoData)),
    ];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (color, label) in items {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(theme::RADIUS_XS),
                    color,
                    theme::stroke(1.0, theme::BORDER_STRONG),
                    StrokeKind::Inside,
                );
                ui.label(RichText::new(label).size(theme::FS_SMALL).color(theme::TEXT_MUTED));
            });
        }
    });
}

// ---------- журнал ----------

impl Severity {
    fn text_color(self) -> Color32 {
        match self {
            Severity::Danger => theme::on_dark(theme::danger()),
            Severity::Warning => theme::on_dark(theme::warning()),
            Severity::Success => theme::on_dark(theme::success()),
            Severity::Info => theme::TEXT,
        }
    }

    fn dot_color(self) -> Color32 {
        match self {
            Severity::Danger => theme::danger(),
            Severity::Warning => theme::warning(),
            Severity::Success => theme::success(),
            Severity::Info => theme::BORDER_STRONG,
        }
    }
}

/// "YYYY-MM-DD HH:MM:SS сообщение" → (метка времени, сообщение).
fn split_log_line(line: &str) -> (&str, &str) {
    match (line.get(..19), line.get(20..)) {
        (Some(ts), Some(msg)) if line.as_bytes().get(19) == Some(&b' ') => (ts, msg),
        _ => ("", line),
    }
}

fn log_job(line: &str) -> LayoutJob {
    let (ts, msg) = split_log_line(line);
    let font = FontId::monospace(theme::FS_MONO);
    let mut job = LayoutJob::default();
    let mut lead = 0.0;
    if !ts.is_empty() {
        job.append(ts, 0.0, TextFormat::simple(font.clone(), theme::TEXT_MUTED));
        lead = 10.0;
    }
    job.append(msg, lead, TextFormat::simple(font, i18n::severity(msg).text_color()));
    job
}

fn fmt_hms(d: TimeDelta) -> String {
    let s = d.num_seconds().max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
