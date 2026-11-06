//! `WebViewDelegate`: события Servo → события ядра.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

use euclid::Scale;
use klin_core::{EngineEvent, Reply, RequestKind};
use klin_shared::{Disposition, LoadStatus, RequestId, TabId, Url};
use servo::{
    AuthenticationRequest, ContextMenu, CreateNewWebViewRequest, EmbedderControl, FilePicker,
    PermissionRequest, RenderingContext, SimpleDialog, WebView, WebViewDelegate, WebViewId,
};

use crate::protocol;

/// Состояние, общее для движка и делегата (всё в главном потоке).
pub(crate) struct Shared {
    pub events: RefCell<VecDeque<EngineEvent>>,
    pub tabs: RefCell<HashMap<WebViewId, TabId>>,
    pub webviews: RefCell<HashMap<TabId, WebView>>,
    pub pending: RefCell<HashMap<RequestId, Pending>>,
    pub visible: RefCell<Option<TabId>>,
}

impl Shared {
    pub fn register(&self, webview: WebView) -> TabId {
        let tab = TabId::next();
        self.tabs.borrow_mut().insert(webview.id(), tab);
        self.webviews.borrow_mut().insert(tab, webview);
        tab
    }

    pub fn unregister(&self, tab: TabId) {
        if let Some(webview) = self.webviews.borrow_mut().remove(&tab) {
            self.tabs.borrow_mut().remove(&webview.id());
        }
        if *self.visible.borrow() == Some(tab) {
            *self.visible.borrow_mut() = None;
        }
    }

    fn tab(&self, webview: &WebView) -> Option<TabId> {
        self.tabs.borrow().get(&webview.id()).copied()
    }

    fn push(&self, event: EngineEvent) {
        self.events.borrow_mut().push_back(event);
    }

    fn request(&self, tab: TabId, kind: RequestKind, pending: Pending) {
        let id = RequestId::next();
        self.pending.borrow_mut().insert(id, pending);
        self.push(EngineEvent::Request { tab, id, kind });
    }
}

/// Запрос Servo, ждущий ответа интерфейса. Если объект отпущен без ответа,
/// Servo сам отвечает значением по умолчанию (отказ).
pub(crate) enum Pending {
    Dialog(SimpleDialog),
    Permission(PermissionRequest),
    Authentication(AuthenticationRequest),
    FilePicker(FilePicker),
    ContextMenu(ContextMenu),
}

impl Pending {
    pub fn answer(self, reply: Reply) {
        match (self, reply) {
            (Pending::Dialog(SimpleDialog::Prompt(mut p)), Reply::Text(text)) => {
                p.set_current_value(&text);
                p.confirm();
            }
            (Pending::Dialog(d), Reply::Ok | Reply::Text(_)) => d.confirm(),
            (Pending::Dialog(d), _) => d.dismiss(),
            (Pending::Permission(p), Reply::Allow) => p.allow(),
            (Pending::Permission(p), _) => p.deny(),
            (Pending::Authentication(a), Reply::Credentials { username, password }) => {
                a.authenticate(username, password)
            }
            (Pending::Authentication(_), _) => {}
            (Pending::FilePicker(mut f), Reply::Files(paths)) => {
                f.select(&paths);
                f.submit();
            }
            (Pending::FilePicker(f), _) => f.dismiss(),
            (Pending::ContextMenu(menu), Reply::ContextMenuItem(index)) => {
                let action = menu
                    .items()
                    .iter()
                    .filter_map(context_menu_action)
                    .nth(index);
                match action {
                    Some(action) => menu.select(action),
                    None => menu.dismiss(),
                }
            }
            (Pending::ContextMenu(menu), _) => menu.dismiss(),
        }
    }
}

fn context_menu_action(item: &servo::ContextMenuItem) -> Option<servo::ContextMenuAction> {
    match item {
        servo::ContextMenuItem::Item {
            action,
            enabled: true,
            ..
        } => Some(*action),
        _ => None,
    }
}

pub(crate) struct Delegate {
    shared: Rc<Shared>,
    rendering_context: Rc<dyn RenderingContext>,
    scale: Cell<f32>,
    this: Weak<Delegate>,
}

impl Delegate {
    pub fn new(
        shared: Rc<Shared>,
        rendering_context: Rc<dyn RenderingContext>,
        scale: f32,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            shared,
            rendering_context,
            scale: Cell::new(scale),
            this: this.clone(),
        })
    }

    pub fn set_scale(&self, scale: f32) {
        self.scale.set(scale);
    }

    fn with_tab(&self, webview: &WebView, f: impl FnOnce(TabId) -> EngineEvent) {
        if let Some(tab) = self.shared.tab(webview) {
            self.shared.push(f(tab));
        }
    }
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, webview: WebView) {
        self.with_tab(&webview, |tab| EngineEvent::NewFrame { tab });
    }

    fn notify_url_changed(&self, webview: WebView, url: Url) {
        let url = protocol::display_url(url);
        self.with_tab(&webview, |tab| EngineEvent::UrlChanged { tab, url });
    }

    fn notify_page_title_changed(&self, webview: WebView, title: Option<String>) {
        let title = title.unwrap_or_default();
        self.with_tab(&webview, |tab| EngineEvent::TitleChanged { tab, title });
    }

    fn notify_status_text_changed(&self, webview: WebView, text: Option<String>) {
        self.with_tab(&webview, |tab| EngineEvent::StatusTextChanged { tab, text });
    }

    fn notify_load_status_changed(&self, webview: WebView, status: servo::LoadStatus) {
        let status = match status {
            servo::LoadStatus::Started => LoadStatus::Started,
            servo::LoadStatus::HeadParsed => LoadStatus::HeadParsed,
            servo::LoadStatus::Complete => LoadStatus::Complete,
        };
        self.with_tab(&webview, |tab| EngineEvent::LoadStatus { tab, status });
    }

    fn notify_favicon_changed(&self, webview: WebView) {
        let icon = webview
            .favicon()
            .and_then(|image| crate::convert::image(&image));
        if let Some(icon) = icon {
            self.with_tab(&webview, |tab| EngineEvent::FaviconChanged { tab, icon });
        }
    }

    fn notify_history_changed(&self, webview: WebView, entries: Vec<Url>, current: usize) {
        let can_back = current > 0;
        let can_forward = current + 1 < entries.len();
        self.with_tab(&webview, |tab| EngineEvent::HistoryChanged {
            tab,
            can_back,
            can_forward,
        });
    }

    fn notify_closed(&self, webview: WebView) {
        self.with_tab(&webview, |tab| EngineEvent::Closed { tab });
    }

    fn notify_crashed(&self, webview: WebView, reason: String, backtrace: Option<String>) {
        if let Some(backtrace) = backtrace {
            log::error!("page crashed: {reason}\n{backtrace}");
        }
        self.with_tab(&webview, |tab| EngineEvent::Crashed { tab, reason });
    }

    fn request_create_new(&self, parent: WebView, request: CreateNewWebViewRequest) {
        let Some(opener) = self.shared.tab(&parent) else {
            return;
        };
        let Some(this) = self.this.upgrade() else {
            return;
        };
        // WebView создаём сразу: Servo требует ответа в этом вызове, чтобы сохранить opener.
        let webview = request
            .builder(self.rendering_context.clone())
            .hidpi_scale_factor(Scale::new(self.scale.get()))
            .delegate(this)
            .build();
        let url = webview
            .url()
            .unwrap_or_else(|| Url::parse("about:blank").expect("valid"));
        let created = self.shared.register(webview);
        self.shared.push(EngineEvent::OpenTabRequested {
            opener,
            url: protocol::display_url(url),
            disposition: Disposition::ForegroundTab,
            created: Some(created),
        });
    }

    fn request_permission(&self, webview: WebView, request: PermissionRequest) {
        let Some(tab) = self.shared.tab(&webview) else {
            return;
        };
        let name = format!("{:?}", request.feature()).to_lowercase();
        self.shared.request(
            tab,
            RequestKind::Permission { name },
            Pending::Permission(request),
        );
    }

    fn request_authentication(&self, webview: WebView, request: AuthenticationRequest) {
        let Some(tab) = self.shared.tab(&webview) else {
            return;
        };
        let kind = RequestKind::Authentication {
            url: request.url().clone(),
            for_proxy: request.for_proxy(),
        };
        self.shared
            .request(tab, kind, Pending::Authentication(request));
    }

    fn show_embedder_control(&self, webview: WebView, control: EmbedderControl) {
        let Some(tab) = self.shared.tab(&webview) else {
            return;
        };
        match control {
            EmbedderControl::SimpleDialog(dialog) => {
                let message = dialog.message().to_owned();
                let kind = match &dialog {
                    SimpleDialog::Alert(_) => RequestKind::Alert { message },
                    SimpleDialog::Confirm(_) => RequestKind::Confirm { message },
                    SimpleDialog::Prompt(p) => RequestKind::Prompt {
                        message,
                        default: p.current_value().to_owned(),
                    },
                };
                self.shared.request(tab, kind, Pending::Dialog(dialog));
            }
            EmbedderControl::FilePicker(picker) => {
                let kind = RequestKind::FilePicker {
                    multiple: picker.allow_select_multiple(),
                    accept: picker
                        .filter_patterns()
                        .iter()
                        .map(|p| format!("{p:?}"))
                        .collect(),
                };
                self.shared.request(tab, kind, Pending::FilePicker(picker));
            }
            EmbedderControl::ContextMenu(menu) => {
                self.shared
                    .request(tab, RequestKind::ContextMenu, Pending::ContextMenu(menu));
            }
            // Выпадающие списки, выбор цвета и IME — этап 0.1, вместе с интерфейсом egui.
            _ => {}
        }
    }
}
