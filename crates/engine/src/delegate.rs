use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};

use euclid::Scale;
use klin_core::{
    EngineEvent, MenuAction, MenuItem, PermissionFeature, Reply, RequestKind, SelectOption,
};
use klin_shared::{Disposition, LoadStatus, RequestId, TabId, Url, WindowId};
use servo::{
    AuthenticationRequest, ContextMenu, ContextMenuAction, ContextMenuItem,
    CreateNewWebViewRequest, DeviceIntRect, EmbedderControl, EmbedderControlId, FilePicker,
    OffscreenRenderingContext, PermissionRequest, RenderingContext, SelectElement,
    SelectElementOptionOrOptgroup, SimpleDialog, WebView, WebViewDelegate, WebViewId,
};

use crate::Surface;

pub(crate) const POPUP_MAX: (f32, f32) = (800.0, 600.0);
pub(crate) const POPUP_MIN: (f32, f32) = (25.0, 25.0);

pub(crate) fn fit_popup(shared: &Rc<Shared>, window: WindowId, webview: &WebView) {
    let Some(surface) = shared.surfaces.borrow().get(&window).cloned() else {
        return;
    };
    let target = webview.clone();
    let script = "JSON.stringify([Math.max(document.documentElement.scrollWidth, document.body ? document.body.scrollWidth : 0), Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0)])";
    webview.evaluate_javascript(script, move |result| {
        let Ok(servo::JSValue::String(json)) = result else {
            return;
        };
        let Ok([width, height]) = serde_json::from_str::<[f32; 2]>(&json) else {
            return;
        };
        let scale = surface.scale.get();
        let width = width.clamp(POPUP_MIN.0, POPUP_MAX.0) * scale;
        let height = height.clamp(POPUP_MIN.1, POPUP_MAX.1) * scale;
        target.resize(dpi::PhysicalSize::new(
            width.ceil() as u32,
            height.ceil() as u32,
        ));
    });
}
use crate::protocol;

#[derive(Default)]
pub(crate) struct Shared {
    pub surfaces: RefCell<HashMap<WindowId, Rc<Surface>>>,
    pub tab_window: RefCell<HashMap<TabId, WindowId>>,
    pub events: RefCell<VecDeque<EngineEvent>>,
    pub tabs: RefCell<HashMap<WebViewId, TabId>>,
    pub webviews: RefCell<HashMap<TabId, WebView>>,
    pub pending: RefCell<HashMap<RequestId, (TabId, Pending)>>,
    pub controls: RefCell<Vec<(EmbedderControlId, TabId, RequestId)>>,
    pub visible: RefCell<HashMap<WindowId, TabId>>,
    pub background_opener: RefCell<Option<TabId>>,
    pub private_windows: RefCell<HashSet<WindowId>>,
    pub blocker: RefCell<Option<klin_content_blocking::Blocker>>,
    pub pdf_viewer_disabled: Cell<bool>,
    pub user_content: RefCell<Option<Rc<servo::UserContentManager>>>,
    pub popups: RefCell<HashMap<WindowId, TabId>>,
    pub hold_requests: Cell<bool>,
    pub held: RefCell<HeldRequests>,
    pub next_request: Cell<u64>,
    pub detached_contexts: RefCell<HashMap<TabId, Rc<OffscreenRenderingContext>>>,
}

impl Shared {
    pub fn register(&self, webview: WebView, window: WindowId, tab: TabId) -> TabId {
        self.tabs.borrow_mut().insert(webview.id(), tab);
        self.webviews.borrow_mut().insert(tab, webview);
        self.tab_window.borrow_mut().insert(tab, window);
        tab
    }

    pub fn register_detached(&self, webview: WebView, tab: TabId) {
        self.tabs.borrow_mut().insert(webview.id(), tab);
        self.webviews.borrow_mut().insert(tab, webview);
    }

    pub fn popup_window(&self, tab: TabId) -> Option<WindowId> {
        self.popups
            .borrow()
            .iter()
            .find(|(_, t)| **t == tab)
            .map(|(w, _)| *w)
    }

    pub fn window_of(&self, tab: TabId) -> Option<WindowId> {
        self.tab_window.borrow().get(&tab).copied()
    }

    pub fn webviews_of(&self, window: WindowId) -> Vec<WebView> {
        let tab_window = self.tab_window.borrow();
        self.webviews
            .borrow()
            .iter()
            .filter(|(tab, _)| tab_window.get(tab) == Some(&window))
            .map(|(_, w)| w.clone())
            .collect()
    }

    pub fn unregister(&self, tab: TabId) {
        if let Some(webview) = self.webviews.borrow_mut().remove(&tab) {
            self.tabs.borrow_mut().remove(&webview.id());
        }
        if let Some(window) = self.tab_window.borrow_mut().remove(&tab) {
            let mut visible = self.visible.borrow_mut();
            if visible.get(&window) == Some(&tab) {
                visible.remove(&window);
            }
        }

        self.pending.borrow_mut().retain(|_, (t, _)| *t != tab);
        self.controls.borrow_mut().retain(|(_, t, _)| *t != tab);
        self.popups.borrow_mut().retain(|_, t| *t != tab);
        self.detached_contexts.borrow_mut().remove(&tab);
    }

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
    this: Weak<Delegate>,
}

impl Delegate {
    pub fn new(shared: Rc<Shared>) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            shared,
            this: this.clone(),
        })
    }

    fn with_tab(&self, webview: &WebView, f: impl FnOnce(TabId) -> EngineEvent) {
        if let Some(tab) = self.shared.tab(webview) {
            self.shared.push(f(tab));
        }
    }
}

pub(crate) type HeldRequests =
    HashMap<u64, (servo::WebResourceLoad, std::time::Instant, Option<TabId>)>;

pub(crate) fn resource_type(destination: &str, main_frame: bool, url: &Url) -> &'static str {
    if main_frame {
        return "main_frame";
    }
    if matches!(url.scheme(), "ws" | "wss") {
        return "websocket";
    }
    match destination {
        "IFrame" | "Frame" => "sub_frame",
        "Script" | "Worker" | "SharedWorker" | "ServiceWorker" | "AudioWorklet"
        | "PaintWorklet" => "script",
        "Style" => "stylesheet",
        "Image" => "image",
        "Font" => "font",
        "Object" | "Embed" => "object",
        "Audio" | "Video" | "Track" => "media",
        "Report" => "csp_report",
        "Xslt" => "xslt",
        "Manifest" => "web_manifest",
        "None" => "xmlhttprequest",
        _ => "other",
    }
}

impl Delegate {
    fn load_resource(&self, webview: WebView, load: servo::WebResourceLoad) {
        let request = load.request();
        if !request.is_for_main_frame {
            let blocked = match (self.shared.blocker.borrow().as_ref(), webview.url()) {
                (Some(blocker), Some(top)) => {
                    let private = self
                        .shared
                        .tab(&webview)
                        .and_then(|tab| self.shared.window_of(tab))
                        .is_some_and(|w| self.shared.private_windows.borrow().contains(&w));
                    blocker.classify(&request.url, &top, private)
                }
                _ => None,
            };
            if let Some(category) = blocked {
                let url = request.url.clone();
                let response = servo::WebResourceResponse::new(url.clone());
                load.intercept(response).cancel();
                if let Some(tab) = self.shared.tab(&webview) {
                    self.shared
                        .push(EngineEvent::ContentBlocked { tab, url, category });
                }
                return;
            }
            self.hold(webview, load);
            return;
        }
        let is_pdf = !self.shared.pdf_viewer_disabled.get()
            && request.method == http::Method::GET
            && matches!(request.url.scheme(), "http" | "https")
            && klin_internal_pages::resources::is_pdf_url(request.url.path());
        if is_pdf {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            let response = servo::WebResourceResponse::new(request.url.clone())
                .headers(headers)
                .status_code(http::StatusCode::OK);
            let mut intercepted = load.intercept(response);
            intercepted.send_body_data(klin_internal_pages::resources::PDF_VIEWER_HTML.to_vec());
            intercepted.finish();
            return;
        }
        let is_download = request.method == http::Method::GET
            && klin_core::downloads::looks_like_download(&request.url);
        if !is_download {
            self.hold(webview, load);
            return;
        }
        let Some(tab) = self.shared.tab(&webview) else {
            return;
        };
        let url = request.url.clone();
        let referrer = request.referrer_url.clone();

        let response =
            servo::WebResourceResponse::new(url.clone()).status_code(http::StatusCode::NO_CONTENT);
        load.intercept(response).finish();
        self.shared
            .push(EngineEvent::DownloadRequested { tab, url, referrer });
    }

    fn hold(&self, webview: WebView, load: servo::WebResourceLoad) {
        if !self.shared.hold_requests.get() {
            return;
        }
        let request = load.request();
        let id = self.shared.next_request.get() + 1;
        self.shared.next_request.set(id);
        let destination = format!("{:?}", request.destination);
        let tab = self.shared.tab(&webview);
        let info = klin_core::WebRequestInfo {
            id,
            tab,
            url: request.url.clone(),
            method: request.method.to_string(),
            kind: resource_type(&destination, request.is_for_main_frame, &request.url),
            document: if request.is_for_main_frame {
                None
            } else {
                webview.url()
            },
            redirect: request.is_redirect,
        };
        self.shared
            .held
            .borrow_mut()
            .insert(id, (load, std::time::Instant::now(), tab));
        self.shared.push(EngineEvent::WebRequest(info));
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
        if status == LoadStatus::Complete
            && let Some(tab) = self.shared.tab(&webview)
            && let Some(window) = self.shared.popup_window(tab)
        {
            fit_popup(&self.shared, window, &webview);
        }
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
        let entries = entries.into_iter().map(protocol::display_url).collect();
        self.with_tab(&webview, |tab| EngineEvent::HistoryChanged {
            tab,
            entries,
            index: current,
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

        let Some(window) = self.shared.window_of(opener) else {
            return;
        };
        let Some(surface) = self.shared.surfaces.borrow().get(&window).cloned() else {
            return;
        };

        let mut builder = request
            .builder(surface.page_context.clone() as Rc<dyn RenderingContext>)
            .hidpi_scale_factor(Scale::new(surface.scale.get()))
            .delegate(this);
        if let Some(user_content) = self.shared.user_content.borrow().clone() {
            builder = builder.user_content_manager(user_content);
        }
        let webview = builder.build();
        let url = webview
            .url()
            .unwrap_or_else(|| Url::parse("about:blank").expect("valid"));
        let created = self.shared.register(webview, window, TabId::next());
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

            _ => {}
        }
    }

    fn show_console_message(
        &self,
        webview: WebView,
        _level: servo::ConsoleLogLevel,
        message: String,
    ) {
        if !message.starts_with(klin_extensions::host::MARK) {
            return;
        }
        if let Some(tab) = self.shared.tab(&webview) {
            self.shared.push(EngineEvent::ExtensionMessage {
                tab,
                url: webview.url(),
                message,
            });
        }
    }

    fn load_web_resource(&self, webview: WebView, load: servo::WebResourceLoad) {
        self.load_resource(webview, load);
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
