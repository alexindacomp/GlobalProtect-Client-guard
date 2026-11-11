//! Языки интерфейса (русский, украинский, английский).
//!
//! Простые строки — ключи `Msg` (полнота переводов проверяется компилятором: у каждого
//! ключа ровно три варианта), строки с параметрами — функции этого модуля.
//! Текущий язык глобальный и читается из любого потока.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Lang {
    Ru = 0,
    Uk = 1,
    En = 2,
}

impl Lang {
    /// Значение `Config::language` → язык (`None` для "auto" и неизвестных значений).
    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "ru" => Some(Lang::Ru),
            "uk" => Some(Lang::Uk),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

/// Значения `Config::language`: код и название языка на нём самом ("auto" переводится).
pub const LANGUAGE_OPTIONS: [(&str, Option<&str>); 4] =
    [("auto", None), ("ru", Some("Русский")), ("uk", Some("Українська")), ("en", Some("English"))];

static LANG: AtomicU8 = AtomicU8::new(Lang::Ru as u8);

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Uk,
        2 => Lang::En,
        _ => Lang::Ru,
    }
}

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

/// Язык интерфейса Windows: украинский, русский, иначе английский.
pub fn detect() -> Lang {
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    match id & 0x3FF {
        0x22 => Lang::Uk,
        0x19 => Lang::Ru,
        _ => Lang::En,
    }
}

/// Применить настройку `Config::language` ("auto" — язык системы).
pub fn apply_setting(code: &str) {
    set_lang(Lang::from_code(code).unwrap_or_else(detect));
}

/// Выбор варианта для текущего языка.
fn pick<T>(ru: T, uk: T, en: T) -> T {
    match lang() {
        Lang::Ru => ru,
        Lang::Uk => uk,
        Lang::En => en,
    }
}

macro_rules! messages {
    ($($key:ident => [$ru:expr, $uk:expr, $en:expr $(,)?]),* $(,)?) => {
        /// Ключи простых (без параметров) строк интерфейса и журнала.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Msg { $($key),* }

        /// Строка на текущем языке.
        pub fn t(m: Msg) -> &'static str {
            match m { $(Msg::$key => pick($ru, $uk, $en)),* }
        }
    };
}

messages! {
    // ---- состояния подключения
    StateStarting => ["Запуск…", "Запуск…", "Starting…"],
    StateConnected => ["Подключено", "Підключено", "Connected"],
    StateDisconnected => ["Нет подключения", "Немає підключення", "Disconnected"],
    StateConnecting => ["Подключение…", "Підключення…", "Connecting…"],
    StatePaused => ["Мониторинг на паузе", "Моніторинг призупинено", "Monitoring paused"],
    StateNoProfile => ["Профиль не заполнен", "Профіль не заповнено", "Profile incomplete"],

    // ---- действия (окно и трей)
    ConnectNow => ["Подключить сейчас", "Підключити зараз", "Connect now"],
    RestartGp => ["Перезапустить GlobalProtect", "Перезапустити GlobalProtect", "Restart GlobalProtect"],
    PauseMonitoring => ["Приостановить мониторинг", "Призупинити моніторинг", "Pause monitoring"],
    ResumeMonitoring => ["Возобновить мониторинг", "Відновити моніторинг", "Resume monitoring"],
    RestartConfirmTitle => ["Перезапустить GlobalProtect?", "Перезапустити GlobalProtect?", "Restart GlobalProtect?"],
    Restart => ["Перезапустить", "Перезапустити", "Restart"],
    Cancel => ["Отмена", "Скасувати", "Cancel"],

    // ---- трей
    TrayOpen => ["Открыть GP Guard", "Відкрити GP Guard", "Open GP Guard"],
    TrayShowGp => ["Показать окно GlobalProtect", "Показати вікно GlobalProtect", "Show GlobalProtect window"],
    TrayQuit => ["Выход", "Вихід", "Exit"],

    // ---- навигация и страницы
    NavCaption => ["НАВИГАЦИЯ", "НАВІГАЦІЯ", "NAVIGATION"],
    BrandSubtitle => ["VPN-монитор", "VPN-монітор", "VPN monitor"],
    PageOverview => ["Обзор", "Огляд", "Overview"],
    PageProfile => ["Профиль", "Профіль", "Profile"],
    PageSettings => ["Настройки", "Налаштування", "Settings"],
    PageLog => ["Журнал", "Журнал", "Log"],
    PageOverviewSub => ["Состояние VPN-туннеля GlobalProtect", "Стан VPN-тунелю GlobalProtect", "GlobalProtect VPN tunnel status"],
    PageProfileSub => [
        "Учётные данные для подключения к порталу",
        "Облікові дані для підключення до порталу",
        "Credentials for connecting to the portal",
    ],
    PageSettingsSub => [
        "Поведение мониторинга и параметры переподключения",
        "Поведінка моніторингу та параметри перепідключення",
        "Monitoring behavior and reconnect settings",
    ],
    PageLogSub => ["Последние события GP Guard", "Останні події GP Guard", "Recent GP Guard events"],

    // ---- панель сохранения
    Unsaved => ["Есть несохранённые изменения", "Є незбережені зміни", "Unsaved changes"],
    AllSaved => ["Все изменения сохранены", "Усі зміни збережено", "All changes saved"],
    Save => ["Сохранить", "Зберегти", "Save"],
    Revert => ["Отменить изменения", "Скасувати зміни", "Discard changes"],
    RevertHint => ["Вернуть сохранённые значения", "Повернути збережені значення", "Revert to the saved values"],
    NoChanges => ["Нет изменений", "Немає змін", "No changes"],
    Saved => ["Сохранено", "Збережено", "Saved"],
    EnterUsername => ["Укажите имя пользователя", "Вкажіть ім’я користувача", "Enter a username"],

    // ---- обзор: карточка статуса
    CaptionState => ["СОСТОЯНИЕ", "СТАН", "STATUS"],
    CaptionTunnelIp => ["IP ТУННЕЛЯ", "IP ТУНЕЛЮ", "TUNNEL IP"],
    DurOnline => ["в сети", "у мережі", "online"],
    DurOffline => ["без связи", "без зв’язку", "disconnected"],
    DurConnecting => ["подключение", "підключення", "connecting"],
    DurPaused => ["на паузе", "на паузі", "paused"],
    Since => ["с", "з", "since"],
    HintNoProfile => [
        "Заполните профиль: без пользователя и пароля GP Guard не сможет подключаться.",
        "Заповніть профіль: без користувача та пароля GP Guard не зможе підключатися.",
        "Complete the profile: GP Guard can't connect without a username and password.",
    ],
    HintPaused => [
        "Мониторинг приостановлен — обрывы связи не обрабатываются автоматически.",
        "Моніторинг призупинено — обриви зв’язку не обробляються автоматично.",
        "Monitoring is paused — connection drops are not handled automatically.",
    ],
    OpenProfile => ["Открыть профиль", "Відкрити профіль", "Open profile"],
    Resume => ["Возобновить", "Відновити", "Resume"],

    // ---- обзор: KPI
    KpiAttempts => ["Попыток подключения", "Спроб підключення", "Connection attempts"],
    KpiRestarts => ["Перезапусков GP", "Перезапусків GP", "GP restarts"],
    KpiHealth => ["Проверка ресурса", "Перевірка ресурсу", "Health check"],
    KpiRunning => ["запущен", "запущено", "running"],
    KpiNotRunning => ["не запущен", "не запущено", "not running"],
    KpiAgent => ["агент GlobalProtect", "агент GlobalProtect", "GlobalProtect agent"],
    KpiSinceStart => ["с момента запуска", "від запуску", "since startup"],
    KpiOn => ["вкл", "увімкнено", "on"],
    KpiOff => ["выкл", "вимкнено", "off"],
    KpiNotSet => ["не задана", "не задано", "not set"],

    // ---- обзор: действия
    Actions => ["Действия", "Дії", "Actions"],
    FillProfile => ["Заполнить профиль", "Заповнити профіль", "Complete profile"],
    ConnectBusyConnected => ["VPN уже подключён.", "VPN вже підключено.", "The VPN is already connected."],
    AlreadyConnecting => ["Подключение уже выполняется", "Підключення вже триває", "Already connecting"],
    ConnectNowHint => [
        "Подключиться к VPN, не дожидаясь очередной проверки",
        "Підключитися до VPN, не чекаючи наступної перевірки",
        "Connect to the VPN without waiting for the next check",
    ],
    RestartGpHint => [
        "Завершить PanGPA.exe, запустить заново и подключиться",
        "Завершити PanGPA.exe, запустити знову й підключитися",
        "Stop PanGPA.exe, start it again and connect",
    ],
    GpWindow => ["Окно GlobalProtect", "Вікно GlobalProtect", "GlobalProtect window"],
    GpWindowHint => ["Показать панель GlobalProtect", "Показати панель GlobalProtect", "Show the GlobalProtect panel"],

    // ---- обзор: история
    NoData => ["нет данных", "немає даних", "no data"],
    Now => ["сейчас", "зараз", "now"],
    LegendConnected => ["подключено", "підключено", "connected"],
    LegendConnecting => ["подключение", "підключення", "connecting"],
    LegendOffline => ["нет связи", "немає зв’язку", "disconnected"],
    LegendPaused => ["пауза / нет профиля", "пауза / немає профілю", "paused / no profile"],

    // ---- обзор: события
    RecentEvents => ["Последние события", "Останні події", "Recent events"],
    FullLog => ["Весь журнал →", "Увесь журнал →", "Full log →"],
    NoEvents => ["Событий пока нет", "Подій поки немає", "No events yet"],

    // ---- профиль
    Account => ["Учётная запись", "Обліковий запис", "Account"],
    AccountSub => [
        "Пароль хранится в диспетчере учётных данных Windows, а не в файле настроек.",
        "Пароль зберігається в диспетчері облікових даних Windows, а не у файлі налаштувань.",
        "The password is stored in Windows Credential Manager, not in the settings file.",
    ],
    Portal => ["Портал", "Портал", "Portal"],
    PortalHelp => [
        "Адрес портала GlobalProtect — вводится, только если GP его спросит.",
        "Адреса порталу GlobalProtect — вводиться, лише якщо GP її запитає.",
        "GlobalProtect portal address — entered only if GP asks for it.",
    ],
    Username => ["Пользователь", "Користувач", "Username"],
    Password => ["Пароль", "Пароль", "Password"],
    PasswordSavedHint => ["сохранён — введите, чтобы заменить", "збережено — введіть, щоб замінити", "saved — type to replace"],
    PasswordNotSaved => ["не сохранён", "не збережено", "not saved"],
    GpPathHelp => [
        "Путь к агенту GlobalProtect, который запускает и контролирует GP Guard.",
        "Шлях до агента GlobalProtect, який запускає й контролює GP Guard.",
        "Path to the GlobalProtect agent that GP Guard starts and monitors.",
    ],

    // ---- настройки
    Interface => ["Интерфейс", "Інтерфейс", "Interface"],
    Language => ["Язык интерфейса", "Мова інтерфейсу", "Interface language"],
    LanguageDesc => [
        "Применяется сразу, сохраняется кнопкой «Сохранить».",
        "Застосовується одразу, зберігається кнопкою «Зберегти».",
        "Applies immediately; click “Save” to keep it.",
    ],
    LanguageAuto => ["Авто (язык системы)", "Авто (мова системи)", "Auto (system language)"],
    Behaviour => ["Поведение", "Поведінка", "Behavior"],
    AutoReconnect => ["Переподключать при обрыве связи", "Перепідключатися в разі обриву зв’язку", "Reconnect when the connection drops"],
    AutoReconnectDesc => [
        "Следить за туннелем и восстанавливать подключение автоматически.",
        "Стежити за тунелем і автоматично відновлювати підключення.",
        "Watch the tunnel and restore the connection automatically.",
    ],
    ConnectOnStart => ["Подключаться при запуске", "Підключатися під час запуску", "Connect on startup"],
    ConnectOnStartDesc => [
        "Поднимать VPN при старте GP Guard, если он ещё не подключён.",
        "Піднімати VPN під час запуску GP Guard, якщо його ще не підключено.",
        "Bring the VPN up when GP Guard starts, if it isn't connected yet.",
    ],
    Autostart => ["Запускать при входе в Windows", "Запускати під час входу в Windows", "Start when signing in to Windows"],
    AutostartDesc => [
        "Задача Планировщика при входе пользователя и запись в автозагрузке. Применяется кнопкой «Сохранить».",
        "Завдання Планувальника під час входу користувача та запис в автозавантаженні. Застосовується кнопкою «Зберегти».",
        "A Task Scheduler sign-in task plus a startup entry. Applied when you click “Save”.",
    ],
    AutostartActive => ["Автозапуск настроен", "Автозапуск налаштовано", "Autostart is set up"],
    AutostartInactive => [
        "Автозапуск сейчас не настроен",
        "Автозапуск зараз не налаштовано",
        "Autostart is not set up right now",
    ],
    AutostartTask => ["Планировщик задач", "Планувальник завдань", "Task Scheduler"],
    AutostartRegistry => ["автозагрузка (реестр)", "автозавантаження (реєстр)", "startup entry (registry)"],
    AutostartChecking => ["проверка…", "перевірка…", "checking…"],
    PasswordSavedBadge => [
        "Пароль сохранён в диспетчере учётных данных Windows",
        "Пароль збережено в диспетчері облікових даних Windows",
        "Password is saved in Windows Credential Manager",
    ],
    PasswordMissingBadge => [
        "Пароль не сохранён — введите его и нажмите «Сохранить»",
        "Пароль не збережено — введіть його й натисніть «Зберегти»",
        "Password is not saved — enter it and click “Save”",
    ],
    StartMinimized => ["Запускаться свёрнутым в трей", "Запускатися згорнутим у трей", "Start minimized to the tray"],
    StartMinimizedDesc => [
        "Не показывать окно при запуске — только значок в трее.",
        "Не показувати вікно під час запуску — лише значок у треї.",
        "Don't show the window on startup — only the tray icon.",
    ],
    Advanced => ["Дополнительно", "Додатково", "Advanced"],
    AdvancedSub => ["Тайминги проверки и переподключения.", "Інтервали перевірки та перепідключення.", "Check and reconnect timings."],
    CheckInterval => ["Интервал проверки", "Інтервал перевірки", "Check interval"],
    CheckIntervalDesc => ["Как часто проверять состояние туннеля.", "Як часто перевіряти стан тунелю.", "How often to check the tunnel state."],
    Grace => ["Ждать самовосстановления GP", "Чекати на самовідновлення GP", "Grace period"],
    GraceDesc => [
        "Сколько ждать после обрыва, прежде чем вмешиваться.",
        "Скільки чекати після обриву, перш ніж втручатися.",
        "How long to wait after a drop before GP Guard steps in.",
    ],
    RetryInterval => ["Пауза между попытками", "Пауза між спробами", "Delay between attempts"],
    RetryIntervalDesc => [
        "Минимальный интервал между попытками подключения.",
        "Мінімальний інтервал між спробами підключення.",
        "Minimum interval between connection attempts.",
    ],
    RestartAfter => ["Перезапуск GP после неудач", "Перезапуск GP після невдач", "Restart GP after failures"],
    RestartAfterDesc => [
        "После скольких неудачных попыток перезапускать PanGPA.exe.",
        "Після скількох невдалих спроб перезапускати PanGPA.exe.",
        "Number of failed attempts before PanGPA.exe is restarted.",
    ],
    SecondsSuffix => [" с", " с", " s"],
    HealthCheck => ["Проверка ресурса (host:port)", "Перевірка ресурсу (host:port)", "Health check (host:port)"],
    HealthCheckHelp => [
        "Если задано, VPN считается рабочим, только когда этот адрес доступен по TCP.",
        "Якщо задано, VPN вважається робочим лише тоді, коли ця адреса доступна по TCP.",
        "If set, the VPN counts as working only while this address is reachable over TCP.",
    ],
    HealthCheckHint => [
        "например, intranet.local:443 — пусто = выкл.",
        "наприклад, intranet.local:443; порожнє поле — вимкнено",
        "e.g. intranet.local:443 — empty = off",
    ],
    Diagnostics => ["Диагностика", "Діагностика", "Diagnostics"],
    DiagnosticsSub => [
        "Выводит в журнал элементы окна GlobalProtect — пригодится, если автоматический вход перестал срабатывать.",
        "Виводить у журнал елементи вікна GlobalProtect — знадобиться, якщо автоматичний вхід перестав спрацьовувати.",
        "Writes the GlobalProtect window's elements to the log — useful if automatic sign-in stops working.",
    ],
    DiagnoseGpWindow => ["Диагностика окна GP", "Діагностика вікна GP", "Diagnose GP window"],
    OpenLog => ["Открыть журнал", "Відкрити журнал", "Open log"],

    // ---- журнал
    FilterHint => ["Фильтр по тексту…", "Фільтр за текстом…", "Filter by text…"],
    ClearFilter => ["Сбросить", "Скинути", "Clear"],
    LogEmpty => ["Журнал пока пуст", "Журнал поки порожній", "The log is empty"],

    // ---- сообщения журнала: супервизор
    LogStarted => ["GP Guard запущен", "GP Guard запущено", "GP Guard started"],
    LogManualConnect => ["ручной запрос подключения", "ручний запит на підключення", "manual connect request"],
    LogAlreadyConnected => ["VPN уже подключён", "VPN вже підключено", "VPN already connected"],
    LogPaused => ["мониторинг приостановлен", "моніторинг призупинено", "monitoring paused"],
    LogResumed => ["мониторинг возобновлён", "моніторинг відновлено", "monitoring resumed"],
    LogSettingsSaved => ["настройки сохранены", "налаштування збережено", "settings saved"],
    LogGpWindowNotFound => ["окно GlobalProtect не найдено", "вікно GlobalProtect не знайдено", "GlobalProtect window not found"],
    LogDiagHeader => ["диагностика панели GP:", "діагностика панелі GP:", "GP panel diagnostics:"],
    LogStartingGp => ["PanGPA.exe не запущен — запускаю", "PanGPA.exe не запущено — запускаю", "PanGPA.exe is not running — starting it"],
    LogReconnectStalled => ["переподключаю зависший туннель", "перепідключаю тунель, що завис", "reconnecting the stalled tunnel"],
    LogVpnDropped => ["обрыв VPN-подключения", "обрив VPN-підключення", "VPN connection dropped"],
    LogNoPassword => ["пароль не сохранён — заполните профиль", "пароль не збережено — заповніть профіль", "password not saved — complete the profile"],
    LogConnecting => ["подключение к VPN…", "підключення до VPN…", "connecting to VPN…"],

    // ---- сообщения журнала: GP
    GpAddressNotResolved => ["адрес не разрешился", "не вдалося розпізнати адресу (DNS)", "address did not resolve"],
    GpPortalEntry => ["ввод портала", "введення порталу", "portal entry"],
    GpUnknownScreen => ["неизвестный экран", "невідомий екран", "unknown screen"],
    GpGatewayConnect => ["Connect (выбор шлюза)", "Connect (вибір шлюзу)", "Connect (gateway selection)"],
    GpUntrustedCert => [
        "GP предупреждает о недоверенном сертификате портала",
        "GP попереджає про недовірений сертифікат порталу",
        "GP warns about an untrusted portal certificate",
    ],
    GpPasswordChange => ["GP требует сменить пароль", "GP вимагає змінити пароль", "GP requires a password change"],
    GpAuthRejected => ["вход не принят", "вхід не прийнято", "login rejected"],
    GpNoPanel => ["не найдена панель GlobalProtect", "не знайдено панель GlobalProtect", "GlobalProtect panel not found"],
    GpPasswordFillFailed => [
        "не удалось вписать пароль в форму GP",
        "не вдалося ввести пароль у форму GP",
        "failed to enter the password into the GP form",
    ],
    GpCredentialsSubmitted => [
        "логин и пароль переданы в форму GP",
        "логін і пароль передано у форму GP",
        "username and password passed to the GP form",
    ],
    GpPortalMissing => [
        "GP запрашивает адрес портала, а в профиле он не указан",
        "GP запитує адресу порталу, але в профілі її не вказано",
        "GP asks for the portal address, but the profile doesn't have one",
    ],
    GpPanelNotFound => ["панель GlobalProtect не найдена", "панель GlobalProtect не знайдено", "GlobalProtect panel not found"],
    GpPasswordPlaceholder => ["<пароль>", "<пароль>", "<password>"],

    // ---- сообщения журнала: прочее
    LogDiagBanner => ["=== диагностика ===", "=== діагностика ===", "=== diagnostics ==="],
    LogTrayExit => ["выход по команде из трея", "вихід за командою з трею", "exit requested from the tray"],
}

// ---------------------------------------------------------------- строки с параметрами

pub fn restart_confirm_text(ip: &str) -> String {
    pick(
        format!("VPN сейчас подключён (IP {ip}). Перезапуск разорвёт туннель; GP Guard подключится заново автоматически."),
        format!("VPN зараз підключено (IP {ip}). Перезапуск розірве тунель; GP Guard автоматично підключиться знову."),
        format!("VPN is connected (IP {ip}). Restarting will drop the tunnel; GP Guard will reconnect automatically."),
    )
}

pub fn version(v: &str) -> String {
    pick(format!("версия {v}"), format!("версія {v}"), format!("version {v}"))
}

pub fn nav_unsaved(page: &str) -> String {
    pick(
        format!("{page} — есть несохранённые изменения"),
        format!("{page} — є незбережені зміни"),
        format!("{page} — unsaved changes"),
    )
}

/// «в сети 01:02:03  ·  с 12:00:00»
pub fn hero_duration(prefix: Msg, elapsed: &str, since: &str) -> String {
    format!("{} {elapsed}  ·  {} {since}", t(prefix), t(Msg::Since))
}

pub fn timeline_title(minutes: i64) -> String {
    pick(
        format!("История подключения за {minutes} мин"),
        format!("Історія підключення за {minutes} хв"),
        format!("Connection history (last {minutes} min)"),
    )
}

/// «в сети 97%  ·  за 42 мин»; `minutes == 0` — меньше минуты.
pub fn timeline_summary(percent: i64, minutes: i64) -> String {
    let span = match (lang(), minutes) {
        (Lang::Ru, 0) => "< 1 мин".to_string(),
        (Lang::Uk, 0) => "< 1 хв".to_string(),
        (Lang::En, 0) => "< 1 min".to_string(),
        (Lang::Ru, m) => format!("{m} мин"),
        (Lang::Uk, m) => format!("{m} хв"),
        (Lang::En, m) => format!("{m} min"),
    };
    pick(
        format!("в сети {percent}%  ·  за {span}"),
        format!("у мережі {percent}%  ·  за {span}"),
        format!("{percent}% online  ·  {span} of data"),
    )
}

pub fn timeline_a11y(summary: &str) -> String {
    pick(
        format!("История подключения: {summary}"),
        format!("Історія підключення: {summary}"),
        format!("Connection history: {summary}"),
    )
}

/// Подпись оси: минуты назад («−60 мин» у левого края, далее просто числа).
pub fn axis_minutes_ago(minutes: i64, with_unit: bool) -> String {
    if !with_unit {
        return format!("−{minutes}");
    }
    pick(format!("−{minutes} мин"), format!("−{minutes} хв"), format!("−{minutes} min"))
}

pub fn data_since(time: &str) -> String {
    pick(
        format!("данные с {time} (запуск GP Guard)"),
        format!("дані з {time} (запуск GP Guard)"),
        format!("data since {time} (GP Guard started)"),
    )
}

pub fn log_count(shown: usize, total: usize) -> String {
    pick(format!("{shown} из {total} строк"), format!("{shown} з {total} рядків"), format!("{shown} of {total} lines"))
}

pub fn log_no_match(filter: &str) -> String {
    pick(
        format!("Нет строк, содержащих «{filter}»"),
        format!("Немає рядків, що містять «{filter}»"),
        format!("No lines containing “{filter}”"),
    )
}

pub fn save_password_failed(e: &str) -> String {
    pick(
        format!("Не удалось сохранить пароль: {e}"),
        format!("Не вдалося зберегти пароль: {e}"),
        format!("Failed to save the password: {e}"),
    )
}

pub fn log_file_write_failed(e: &str) -> String {
    pick(
        format!("не удалось записать в файл журнала: {e}"),
        format!("не вдалося записати у файл журналу: {e}"),
        format!("failed to write to the log file: {e}"),
    )
}

pub fn autostart_failed(e: &str) -> String {
    pick(format!("Автозапуск: {e}"), format!("Автозапуск: {e}"), format!("Autostart: {e}"))
}

pub fn write_settings_failed(e: &str) -> String {
    pick(
        format!("Не удалось записать настройки: {e}"),
        format!("Не вдалося записати налаштування: {e}"),
        format!("Failed to write the settings: {e}"),
    )
}

// ---- журнал: супервизор

pub fn log_vpn_connected(ip: &str) -> String {
    pick(format!("VPN подключён, IP {ip}"), format!("VPN підключено, IP {ip}"), format!("VPN connected, IP {ip}"))
}

pub fn log_health_failing(e: &str) -> String {
    pick(
        format!("туннель поднят, но проверка не проходит: {e}"),
        format!("тунель піднято, але перевірка не проходить: {e}"),
        format!("tunnel is up, but the health check fails: {e}"),
    )
}

pub fn log_restarting_gp(n: usize) -> String {
    pick(
        format!("перезапуск GlobalProtect (завершено процессов: {n})"),
        format!("перезапуск GlobalProtect (завершено процесів: {n})"),
        format!("restarting GlobalProtect (processes terminated: {n})"),
    )
}

pub fn log_auto_retry_stopped(e: &str) -> String {
    pick(
        format!("{e}. Автоповтор остановлен до «Подключить сейчас» или сохранения профиля"),
        format!("{e}. Автоповтор зупинено до «Підключити зараз» або збереження профілю"),
        format!("{e}. Automatic retries stopped until “Connect now” or the profile is saved"),
    )
}

pub fn log_attempt_failed(n: u32, e: &str) -> String {
    pick(
        format!("попытка не удалась ({n}): {e}"),
        format!("спроба не вдалася ({n}): {e}"),
        format!("attempt failed ({n}): {e}"),
    )
}

// ---- журнал: GP

pub fn gp_start_failed(path: &str, e: &str) -> String {
    pick(
        format!("не удалось запустить {path}: {e}"),
        format!("не вдалося запустити {path}: {e}"),
        format!("failed to start {path}: {e}"),
    )
}

pub fn gp_status_screen(text: &str, button: &str) -> String {
    pick(
        format!("статус «{text}», кнопка «{button}»"),
        format!("статус «{text}», кнопка «{button}»"),
        format!("status “{text}”, button “{button}”"),
    )
}

pub fn gp_login_screen(message: &str) -> String {
    pick(format!("форма входа «{message}»"), format!("форма входу «{message}»"), format!("login form “{message}”"))
}

pub fn gp_click_screen(what: &str) -> String {
    pick(
        format!("экран с кнопкой «{what}»"),
        format!("екран із кнопкою «{what}»"),
        format!("screen with the “{what}” button"),
    )
}

pub fn gp_auth_rejected_msg(m: &str) -> String {
    pick(format!("вход не принят: «{m}»"), format!("вхід не прийнято: «{m}»"), format!("login rejected: “{m}”"))
}

pub fn gp_needs_user(s: &str) -> String {
    pick(
        format!("{s} — требуется действие пользователя"),
        format!("{s} — потрібна дія користувача"),
        format!("{s} — user action required"),
    )
}

pub fn gp_timeout(s: &str) -> String {
    pick(format!("таймаут: {s}"), format!("тайм-аут: {s}"), format!("timeout: {s}"))
}

pub fn gp_pressing(what: &str) -> String {
    pick(format!("GP: нажимаю «{what}»"), format!("GP: натискаю «{what}»"), format!("GP: clicking “{what}”"))
}

pub fn gp_portal_entered(portal: &str) -> String {
    pick(format!("введён портал {portal}"), format!("введено портал {portal}"), format!("portal entered: {portal}"))
}

pub fn gp_tunnel_not_up(secs: u64) -> String {
    pick(
        format!("туннель не поднялся за {secs} с"),
        format!("тунель не піднявся за {secs} с"),
        format!("tunnel did not come up within {secs} s"),
    )
}

pub fn gp_dump_panel(hwnd: usize, screen: &str) -> String {
    pick(
        format!("панель 0x{hwnd:X}, экран: {screen}"),
        format!("панель 0x{hwnd:X}, екран: {screen}"),
        format!("panel 0x{hwnd:X}, screen: {screen}"),
    )
}

pub fn gp_dump_page(i: usize) -> String {
    pick(format!("страница #{i}:"), format!("сторінка #{i}:"), format!("page #{i}:"))
}

// ---- журнал: прочее

pub fn log_diag_gp_running(v: bool) -> String {
    pick(format!("PanGPA запущен: {v}"), format!("PanGPA запущено: {v}"), format!("PanGPA running: {v}"))
}

pub fn log_diag_tunnel_ip(ip: &Option<String>) -> String {
    pick(format!("IP туннеля: {ip:?}"), format!("IP тунелю: {ip:?}"), format!("Tunnel IP: {ip:?}"))
}

pub fn log_diag_gp_window(found: bool) -> String {
    pick(format!("окно GP найдено: {found}"), format!("вікно GP знайдено: {found}"), format!("GP window found: {found}"))
}

pub fn log_autostart_update_failed(e: &str) -> String {
    pick(
        format!("не удалось обновить автозапуск: {e}"),
        format!("не вдалося оновити автозапуск: {e}"),
        format!("failed to update autostart: {e}"),
    )
}

pub fn log_tray_icon_failed(e: &str) -> String {
    pick(
        format!("не удалось создать иконку в трее: {e}"),
        format!("не вдалося створити значок у треї: {e}"),
        format!("failed to create the tray icon: {e}"),
    )
}

// ---------------------------------------------------------------- важность строк журнала

/// Важность строки журнала по ключевым словам. Проверяются все языки сразу:
/// в журнале могут быть строки, записанные до смены языка.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Danger,
    Warning,
    Success,
    Info,
}

pub fn severity(msg: &str) -> Severity {
    const DANGER: [&str; 12] = [
        "не удал", "ошибк", "не принят", "не поднялся", // ru
        "не вдал", "помилк", "не прийнят", "не піднявся", // uk
        "failed", "error", "rejected", "did not come up", // en
    ];
    const WARNING: [&str; 21] = [
        "таймаут", "обрыв", "не проходит", "не найден", "не запущен", "не сохранён", "требуется действие", // ru
        "тайм-аут", "обрив", "не проходить", "не знайден", "не запущено", "не збережено", "потрібна дія", // uk
        "timeout", "dropped", "fails", "not found", "not running", "not saved", "action required", // en
    ];
    const SUCCESS: [&str; 5] = ["подключён", "vpn підключено", "vpn вже підключено", "vpn connected", "vpn already connected"];
    // Причина внутри сообщения, а не его итог: «не вдалося…» здесь не должно красить
    // строку проверки ресурса в красный (в русском «адрес не разрешился» — без ключевых слов).
    const NEUTRAL: [&str; 1] = ["не вдалося розпізнати адресу"];
    let mut l = msg.to_lowercase();
    for p in NEUTRAL {
        l = l.replace(p, "");
    }
    if DANGER.iter().any(|k| l.contains(k)) {
        Severity::Danger
    } else if WARNING.iter().any(|k| l.contains(k)) {
        Severity::Warning
    } else if SUCCESS.iter().any(|k| l.contains(k)) {
        Severity::Success
    } else {
        Severity::Info
    }
}
