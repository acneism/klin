//! `WebViewDelegate`: события Servo → события ядра.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

use euclid::Scale;
use klin_core::{
    EngineEvent, MenuAction, MenuItem, PermissionFeature, Reply, RequestKind, SelectOption,
};
use klin_shared::{Disposition, LoadStatus, RequestId, TabId, Url};
use servo::{
    AuthenticationRequest, ContextMenu, ContextMenuAction, ContextMenuItem,
    CreateNewWebViewRequest, DeviceIntRect, EmbedderControl, EmbedderControlId, FilePicker,
    PermissionRequest, RenderingContext, SelectElement, SelectElementOptionOrOptgroup,
    SimpleDialog, WebView, WebViewDelegate, WebViewId,
};

use crate::protocol;

/// Состояние, общее для движка и делегата (всё в главном потоке).
pub(crate) struct Shared {
    pub events: RefCell<VecDeque<EngineEvent>>,
    pub tabs: RefCell<HashMap<WebViewId, TabId>>,
    pub webviews: RefCell<HashMap<TabId, WebView>>,
    pub pending: RefCell<HashMap<RequestId, (TabId, Pending)>>,
    /// Элементы управления, которые Servo может снять сам (`hide_embedder_control`).
    pub controls: RefCell<Vec<(EmbedderControlId, TabId, RequestId)>>,
    pub visible: RefCell<Option<TabId>>,
    /// Вкладка, из которой выбрали «Открыть ссылку в новой вкладке»: следующая вкладка
    /// от неё открывается в фоне (`browser.tabs.loadInBackground`).
    pub background_opener: RefCell<Option<TabId>>,
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
        // Отпущенные запросы Servo закрывает сам со значением по умолчанию.
        self.pending.borrow_mut().retain(|_, (t, _)| *t != tab);
        self.controls.borrow_mut().retain(|(_, t, _)| *t != tab);
    }

    /// Снять запрос с учёта, когда на него ответили.
    pub fn take(&self, id: RequestId) -> Option<(TabId, Pending)> {
        self.controls.borrow_mut().retain(|(_, _, r)| *r != id);
        self.pending.borrow_mut().remove(&id)
    }

    fn tab(&self, webview: &WebView) -> Option<TabId> {
        self.tabs.borrow().get(&webview.id()).copied()
    }

    fn push(&self, event: EngineEvent) {
        self.events.borrow_mut().push_back(event);
    }

    fn request(&self, tab: TabId, kind: RequestKind, pending: Pending) -> RequestId {
        let id = RequestId::next();
        self.pending.borrow_mut().insert(id, (tab, pending));
        self.push(EngineEvent::Request { tab, id, kind });
        id
    }

    fn control(&self, tab: TabId, control: EmbedderControlId, kind: RequestKind, pending: Pending) {
        let id = self.request(tab, kind, pending);
        self.controls.borrow_mut().push((control, tab, id));
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
    Select(SelectElement),
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
            (Pending::ContextMenu(menu), Reply::Menu(action)) => {
                menu.select(to_servo_action(action))
            }
            (Pending::ContextMenu(menu), _) => menu.dismiss(),
            (Pending::Select(mut select), Reply::Select(ids)) => {
                select.select(ids);
                select.submit();
            }
            // Закрытый без выбора список оставляет прежнее значение.
            (Pending::Select(select), _) => select.submit(),
        }
    }
}

fn menu_action(action: ContextMenuAction) -> MenuAction {
    match action {
        ContextMenuAction::GoBack => MenuAction::Back,
        ContextMenuAction::GoForward => MenuAction::Forward,
        ContextMenuAction::Reload => MenuAction::Reload,
        ContextMenuAction::CopyLink => MenuAction::CopyLink,
        ContextMenuAction::OpenLinkInNewWebView => MenuAction::OpenLinkInNewTab,
        ContextMenuAction::CopyImageLink => MenuAction::CopyImageLink,
        ContextMenuAction::OpenImageInNewView => MenuAction::OpenImageInNewTab,
        ContextMenuAction::Cut => MenuAction::Cut,
        ContextMenuAction::Copy => MenuAction::Copy,
        ContextMenuAction::Paste => MenuAction::Paste,
        ContextMenuAction::SelectAll => MenuAction::SelectAll,
    }
}

fn to_servo_action(action: MenuAction) -> ContextMenuAction {
    match action {
        MenuAction::Back => ContextMenuAction::GoBack,
        MenuAction::Forward => ContextMenuAction::GoForward,
        MenuAction::Reload => ContextMenuAction::Reload,
        MenuAction::CopyLink => ContextMenuAction::CopyLink,
        MenuAction::OpenLinkInNewTab => ContextMenuAction::OpenLinkInNewWebView,
        MenuAction::CopyImageLink => ContextMenuAction::CopyImageLink,
        MenuAction::OpenImageInNewTab => ContextMenuAction::OpenImageInNewView,
        MenuAction::Cut => ContextMenuAction::Cut,
        MenuAction::Copy => ContextMenuAction::Copy,
        MenuAction::Paste => ContextMenuAction::Paste,
        MenuAction::SelectAll => ContextMenuAction::SelectAll,
    }
}

fn control_kind(control: &EmbedderControl) -> &'static str {
    match control {
        EmbedderControl::SelectElement(_) => "select",
        EmbedderControl::ColorPicker(_) => "color picker",
        EmbedderControl::FilePicker(_) => "file picker",
        EmbedderControl::InputMethod(_) => "input method",
        EmbedderControl::SimpleDialog(_) => "dialog",
        EmbedderControl::ContextMenu(_) => "context menu",
    }
}

fn rect(r: DeviceIntRect) -> [i32; 4] {
    [r.min.x, r.min.y, r.width(), r.height()]
}

fn select_options(options: &[SelectElementOptionOrOptgroup]) -> Vec<SelectOption> {
    let option = |o: &servo::SelectElementOption, group: Option<&String>| SelectOption {
        id: o.id,
        label: o.label.clone(),
        disabled: o.is_disabled,
        group: group.filter(|g| !g.is_empty()).cloned(),
    };
    options
        .iter()
        .flat_map(|entry| match entry {
            SelectElementOptionOrOptgroup::Option(o) => vec![option(o, None)],
            SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                options.iter().map(|o| option(o, Some(label))).collect()
            }
        })
        .collect()
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
        let background = self
            .shared
            .background_opener
            .borrow_mut()
            .take_if(|t| *t == opener)
            .is_some();
        let disposition = if background {
            Disposition::BackgroundTab
        } else {
            Disposition::ForegroundTab
        };
        self.shared.push(EngineEvent::OpenTabRequested {
            opener,
            url: protocol::display_url(url),
            disposition,
            created: Some(created),
        });
    }

    fn request_permission(&self, webview: WebView, request: PermissionRequest) {
        let Some(tab) = self.shared.tab(&webview) else {
            return;
        };
        let feature = match request.feature() {
            servo::PermissionFeature::Geolocation => PermissionFeature::Geolocation,
            servo::PermissionFeature::Notifications | servo::PermissionFeature::Push => {
                PermissionFeature::Notifications
            }
            servo::PermissionFeature::Camera => PermissionFeature::Camera,
            servo::PermissionFeature::Microphone => PermissionFeature::Microphone,
            servo::PermissionFeature::PersistentStorage => PermissionFeature::PersistentStorage,
            _ => PermissionFeature::Other,
        };
        self.shared.request(
            tab,
            RequestKind::Permission { feature },
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
        let control_id = control.id();
        log::debug!("show_embedder_control: {}", control_kind(&control));
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
                self.shared
                    .control(tab, control_id, kind, Pending::Dialog(dialog));
            }
            EmbedderControl::FilePicker(picker) => {
                let kind = RequestKind::FilePicker {
                    multiple: picker.allow_select_multiple(),
                    accept: picker
                        .filter_patterns()
                        .iter()
                        .map(|p| p.0.clone())
                        .collect(),
                };
                self.shared
                    .control(tab, control_id, kind, Pending::FilePicker(picker));
            }
            EmbedderControl::ContextMenu(menu) => {
                let items = menu
                    .items()
                    .iter()
                    .map(|item| match item {
                        ContextMenuItem::Item {
                            action, enabled, ..
                        } => MenuItem::Action {
                            action: menu_action(*action),
                            enabled: *enabled,
                        },
                        ContextMenuItem::Separator => MenuItem::Separator,
                    })
                    .collect();
                let info = menu.element_info();
                let [x, y, _, _] = rect(menu.position());
                let kind = RequestKind::ContextMenu {
                    x,
                    y,
                    items,
                    link: info.link_url.clone(),
                    image: info.image_url.clone(),
                };
                self.shared
                    .control(tab, control_id, kind, Pending::ContextMenu(menu));
            }
            EmbedderControl::SelectElement(select) => {
                let kind = RequestKind::Select {
                    rect: rect(select.position()),
                    options: select_options(select.options()),
                    multiple: select.allow_select_multiple(),
                    selected: select.selected_options(),
                };
                self.shared
                    .control(tab, control_id, kind, Pending::Select(select));
            }
            // Выбор цвета и IME — позже; отпущенный запрос Servo закрывает сам.
            _ => {}
        }
    }

    fn hide_embedder_control(&self, _webview: WebView, control_id: EmbedderControlId) {
        let found = {
            let controls = self.shared.controls.borrow();
            controls
                .iter()
                .find(|(c, _, _)| *c == control_id)
                .map(|(_, t, r)| (*t, *r))
        };
        log::debug!("hide_embedder_control: known = {}", found.is_some());
        if let Some((tab, id)) = found {
            self.shared.take(id);
            self.shared.push(EngineEvent::RequestCancelled { tab, id });
        }
    }
}
