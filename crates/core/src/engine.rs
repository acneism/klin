use klin_shared::{Disposition, Image, LoadStatus, RequestId, TabId, Url, WindowId};

use crate::input::InputEvent;

pub trait Engine {
    fn open_tab(&mut self, tab: TabId, window: WindowId, url: &Url, private: bool);
    fn close_tab(&mut self, tab: TabId);
    fn load(&mut self, tab: TabId, url: &Url);
    fn go_back(&mut self, tab: TabId);
    fn go_forward(&mut self, tab: TabId);
    fn reload(&mut self, tab: TabId, bypass_cache: bool);
    fn stop(&mut self, tab: TabId);
    fn set_zoom(&mut self, tab: TabId, factor: f32);

    fn set_visible(&mut self, tab: TabId, visible: bool);
    fn send_input(&mut self, tab: TabId, input: InputEvent);

    fn find(&mut self, tab: TabId, request: FindRequest);
    fn check_readerable(&mut self, tab: TabId);
    fn detect_login_form(&mut self, tab: TabId);
    fn capture_login(&mut self, tab: TabId);
    fn fill_login(&mut self, tab: TabId, origin: &str, username: &str, password: &str);
    fn show_reader(&mut self, tab: TabId);

    fn answer(&mut self, request: RequestId, reply: Reply);
}

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
        entries: Vec<Url>,
        index: usize,
    },
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
    RequestCancelled {
        tab: TabId,
        id: RequestId,
    },
    NewFrame {
        tab: TabId,
    },
    FindResult {
        tab: TabId,
        result: FindResult,
    },
    Readerable {
        tab: TabId,
        readerable: bool,
    },
    LoginFormFound {
        tab: TabId,
        origin: String,
    },
    LoginCaptured {
        tab: TabId,
        login: klin_shared::logins::LoginInput,
    },
    ContentBlocked {
        tab: TabId,
        url: Url,
        category: klin_shared::tracking::BlockCategory,
    },
    DownloadRequested {
        tab: TabId,
        url: Url,
        referrer: Option<Url>,
    },
    Closed {
        tab: TabId,
    },
    Crashed {
        tab: TabId,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindRequest {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub direction: FindDirection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindDirection {
    First,
    Next,
    Previous,
    Clear,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindResult {
    pub current: usize,
    pub total: usize,
    pub wrapped: Option<FindWrap>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindWrap {
    FromTop,
    FromBottom,
}

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
    Permission {
        feature: PermissionFeature,
    },
    Authentication {
        url: Url,
        for_proxy: bool,
    },
    FilePicker {
        multiple: bool,
        accept: Vec<String>,
    },
    ContextMenu {
        x: i32,
        y: i32,
        items: Vec<MenuItem>,
        link: Option<Url>,
        image: Option<Url>,
    },
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
    pub id: usize,
    pub label: String,
    pub disabled: bool,
    pub group: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Dismiss,
    Ok,
    Text(String),
    Allow,
    Deny,
    Credentials { username: String, password: String },
    Files(Vec<std::path::PathBuf>),
    Menu(MenuAction),
    Select(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageRequest {
    pub id: RequestId,
    pub kind: RequestKind,
}
