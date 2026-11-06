//! Граница между ядром и движком.

use klin_shared::{Disposition, Image, LoadStatus, RequestId, TabId, Url, WindowId};

use crate::input::InputEvent;

/// Всё, что ядро знает о движке. Реализации: `klin_engine::ServoEngine`
/// и [`crate::testing::FakeEngine`] для тестов без Servo.
pub trait Engine {
    fn open_tab(&mut self, window: WindowId, url: &Url, private: bool) -> TabId;
    fn close_tab(&mut self, tab: TabId);
    fn load(&mut self, tab: TabId, url: &Url);
    fn go_back(&mut self, tab: TabId);
    fn go_forward(&mut self, tab: TabId);
    fn reload(&mut self, tab: TabId, bypass_cache: bool);
    fn stop(&mut self, tab: TabId);
    fn set_zoom(&mut self, tab: TabId, factor: f32);
    /// Показать вкладку в окне (выбранная вкладка) или спрятать её.
    fn set_visible(&mut self, tab: TabId, visible: bool);
    fn send_input(&mut self, tab: TabId, input: InputEvent);
    /// Ответ на запрос страницы: диалог, разрешение, файл, меню.
    fn answer(&mut self, request: RequestId, reply: Reply);
}

/// Что произошло в странице (engine → core).
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    LoadStatus {
        tab: TabId,
        status: LoadStatus,
    },
    UrlChanged {
        tab: TabId,
        url: Url,
    },
    TitleChanged {
        tab: TabId,
        title: String,
    },
    FaviconChanged {
        tab: TabId,
        icon: Image,
    },
    StatusTextChanged {
        tab: TabId,
        text: Option<String>,
    },
    HistoryChanged {
        tab: TabId,
        can_back: bool,
        can_forward: bool,
    },
    /// Страница открыла вкладку (window.open, target=_blank). Если движок уже создал
    /// WebView сам (так делает Servo, чтобы сохранить связь с opener), `created` — её TabId,
    /// и ядро не вызывает [`Engine::open_tab`].
    OpenTabRequested {
        opener: TabId,
        url: Url,
        disposition: Disposition,
        created: Option<TabId>,
    },
    Request {
        tab: TabId,
        id: RequestId,
        kind: RequestKind,
    },
    /// Движок сам снял запрос (например, `<select>` убран со страницы).
    RequestCancelled {
        tab: TabId,
        id: RequestId,
    },
    NewFrame {
        tab: TabId,
    },
    Closed {
        tab: TabId,
    },
    Crashed {
        tab: TabId,
        reason: String,
    },
}

/// Вид запроса страницы к встраивателю.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestKind {
    Alert {
        message: String,
    },
    Confirm {
        message: String,
    },
    Prompt {
        message: String,
        default: String,
    },
    /// Разрешение (камера, геолокация, уведомления…).
    Permission {
        feature: PermissionFeature,
    },
    Authentication {
        url: Url,
        for_proxy: bool,
    },
    FilePicker {
        multiple: bool,
        /// Фильтры из атрибута `accept`: расширения (`.png`) или MIME-типы.
        accept: Vec<String>,
    },
    /// Контекстное меню страницы. Координаты — в пикселях устройства относительно страницы.
    ContextMenu {
        x: i32,
        y: i32,
        items: Vec<MenuItem>,
        link: Option<Url>,
        image: Option<Url>,
    },
    /// Выпадающий список `<select>`. `rect` — элемент на странице: x, y, ширина, высота
    /// в пикселях устройства.
    Select {
        rect: [i32; 4],
        options: Vec<SelectOption>,
        multiple: bool,
        selected: Vec<usize>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionFeature {
    Geolocation,
    Notifications,
    Camera,
    Microphone,
    PersistentStorage,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Action { action: MenuAction, enabled: bool },
    Separator,
}

/// Пункты контекстного меню, которые умеет выполнять движок.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Back,
    Forward,
    Reload,
    CopyLink,
    OpenLinkInNewTab,
    CopyImageLink,
    OpenImageInNewTab,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    /// Идентификатор, который возвращается в [`Reply::Select`].
    pub id: usize,
    pub label: String,
    pub disabled: bool,
    /// Метка `<optgroup>`, если пункт в группе.
    pub group: Option<String>,
}

/// Ответ интерфейса на [`RequestKind`].
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// Закрыть alert, «Отмена» в confirm/prompt, отказ в разрешении, закрытое меню.
    Dismiss,
    Ok,
    Text(String),
    Allow,
    Deny,
    Credentials {
        username: String,
        password: String,
    },
    Files(Vec<std::path::PathBuf>),
    Menu(MenuAction),
    Select(Vec<usize>),
}

/// Запрос страницы, ждущий ответа.
#[derive(Clone, Debug, PartialEq)]
pub struct PageRequest {
    pub id: RequestId,
    pub kind: RequestKind,
}
