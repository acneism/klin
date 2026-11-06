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
    /// Разрешение (камера, геолокация, уведомления…), имя — как в Permissions API.
    Permission {
        name: String,
    },
    Authentication {
        url: Url,
        for_proxy: bool,
    },
    FilePicker {
        multiple: bool,
        accept: Vec<String>,
    },
    ContextMenu,
    Unload,
}

/// Ответ интерфейса на [`RequestKind`].
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// Закрыть alert, «Отмена» в confirm/prompt, отказ в разрешении.
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
    ContextMenuItem(usize),
}
