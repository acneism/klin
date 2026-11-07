mod convert;
mod delegate;
mod protocol;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use euclid::Scale;
use klin_core::{Engine, EngineEvent, FindDirection, FindRequest, InputEvent, MenuAction, Reply};
use klin_shared::{RequestId, TabId, Url, WindowId};
use log::warn;
use raw_window_handle::{DisplayHandle, WindowHandle};
use servo::{
    EventLoopWaker, OffscreenRenderingContext, RenderingContext, Servo, ServoBuilder, WebView,
    WebViewBuilder, WindowRenderingContext,
};

use crate::delegate::{Delegate, Pending, Shared};

pub use crate::protocol::internal_url;

#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        (self.0)()
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    pub data_dir: Option<std::path::PathBuf>,
    pub http_cache: Option<std::path::PathBuf>,
}

pub type PageBlitter = Box<dyn Fn(&glow::Context, [i32; 4]) + Send + Sync>;

const FIND_JS: &str = include_str!("../js/find.js");
const READABILITY_JS: &str = include_str!("../js/readability/Readability.js");
const READERABLE_LIB_JS: &str = include_str!("../js/readability/Readability-readerable.js");
const READER_JS: &str = include_str!("../js/reader.js");
const READERABLE_JS: &str = include_str!("../js/readerable.js");

fn wrap_script(library: &str, body: &str) -> String {
    format!(
        "(() => {{\n{library}\nreturn {};\n}})()",
        body.trim().trim_end_matches(';')
    )
}

fn parse_article(json: &str, url: &Url) -> Option<klin_internal_pages::reader::Article> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some(klin_internal_pages::reader::Article {
        url: url.to_string(),
        title: text("title").unwrap_or_default(),
        byline: text("byline"),
        content: text("content")?,
        lang: text("lang"),
        dir: text("dir"),
        length: value.get("length").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
    })
}

fn parse_find_result(json: &str) -> Option<klin_core::FindResult> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let wrapped = match value.get("wrapped").and_then(|w| w.as_str()) {
        Some("top") => Some(klin_core::FindWrap::FromTop),
        Some("bottom") => Some(klin_core::FindWrap::FromBottom),
        _ => None,
    };
    Some(klin_core::FindResult {
        current: value.get("current")?.as_u64()? as usize,
        total: value.get("total")?.as_u64()? as usize,
        wrapped,
    })
}

fn non_zero(width: u32, height: u32) -> dpi::PhysicalSize<u32> {
    dpi::PhysicalSize::new(width.max(1), height.max(1))
}

#[derive(Debug)]
pub struct EngineError(String);

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EngineError {}

pub(crate) struct Surface {
    window_context: Rc<WindowRenderingContext>,
    pub page_context: Rc<OffscreenRenderingContext>,
    pub scale: Cell<f32>,
}

pub struct ServoEngine {
    servo: Servo,
    shared: Rc<Shared>,
    delegate: Rc<Delegate>,
    reader: protocol::ReaderStore,
}

impl ServoEngine {
    pub fn new(waker: Waker, config: EngineConfig) -> Result<Self, EngineError> {
        let reader = protocol::ReaderStore::default();
        let mut protocols = servo::protocol_handler::ProtocolRegistry::with_internal_protocols();
        protocols
            .register(
                klin_internal_pages::SCHEME,
                protocol::InternalPages {
                    reader: reader.clone(),
                },
            )
            .map_err(|e| EngineError(format!("register {}: {e:?}", klin_internal_pages::SCHEME)))?;

        let opts = servo::Opts {
            config_dir: config.data_dir,
            ..Default::default()
        };
        let mut preferences = servo::Preferences::default();
        if let Some(cache) = config.http_cache {
            preferences.network_http_disk_cache = cache.to_string_lossy().into_owned();
        }
        let servo = ServoBuilder::default()
            .opts(opts)
            .preferences(preferences)
            .event_loop_waker(Box::new(waker))
            .protocol_registry(protocols)
            .build();
        servo.setup_logging();

        let shared = Rc::new(Shared::default());
        let delegate = Delegate::new(shared.clone());
        Ok(Self {
            servo,
            shared,
            delegate,
            reader,
        })
    }

    pub fn add_window(
        &mut self,
        window: WindowId,
        display: DisplayHandle<'_>,
        handle: WindowHandle<'_>,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(), EngineError> {
        let window_context = Rc::new(
            WindowRenderingContext::new(display, handle, dpi::PhysicalSize::new(size.0, size.1))
                .map_err(|e| EngineError(format!("rendering context: {e:?}")))?,
        );
        window_context
            .make_current()
            .map_err(|e| EngineError(format!("make_current: {e:?}")))?;
        let page_context = Rc::new(window_context.offscreen_context(non_zero(size.0, size.1)));
        let surface = Surface {
            window_context,
            page_context,
            scale: Cell::new(scale),
        };
        self.shared
            .surfaces
            .borrow_mut()
            .insert(window, Rc::new(surface));
        Ok(())
    }

    pub fn remove_window(&mut self, window: WindowId) {
        self.shared.surfaces.borrow_mut().remove(&window);
        self.shared.visible.borrow_mut().remove(&window);
    }

    pub fn import_cookies(&self, cookies: Vec<klin_shared::import::ImportedCookie>) -> usize {
        use klin_shared::import::SameSite;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let mut imported = 0;
        for c in cookies {
            let host = c.host.trim_start_matches('.');
            let scheme = if c.secure { "https" } else { "http" };
            let path = if c.path.starts_with('/') {
                c.path.clone()
            } else {
                format!("/{}", c.path)
            };
            let Ok(url) = Url::parse(&format!("{scheme}://{host}{path}")) else {
                continue;
            };
            let mut builder = cookie::Cookie::build((c.name, c.value))
                .path(path)
                .secure(c.secure)
                .http_only(c.http_only)
                .same_site(match c.same_site {
                    SameSite::None => cookie::SameSite::None,
                    SameSite::Lax => cookie::SameSite::Lax,
                    SameSite::Strict => cookie::SameSite::Strict,
                });
            if c.host.starts_with('.') {
                builder = builder.domain(host.to_owned());
            }
            if let Some(expires) = c.expires_ms {
                if expires <= now_ms {
                    continue;
                }
                let seconds = ((expires - now_ms) / 1000) as i64;
                builder = builder.max_age(cookie::time::Duration::seconds(seconds));
            }
            self.servo.site_data_manager().set_cookie_for_url(
                url,
                builder.build(),
                Some(Box::new(|| {})),
            );
            imported += 1;
        }
        imported
    }

    pub fn user_agent(&self) -> String {
        servo::pref!(user_agent)
    }

    pub fn cookie_header(&self, url: &Url, callback: impl FnOnce(Option<String>) + 'static) {
        self.servo.site_data_manager().cookies_for_url_async(
            url.clone(),
            servo::CookieSource::HTTP,
            move |cookies| {
                let header = cookies
                    .iter()
                    .map(|c| format!("{}={}", c.name(), c.value()))
                    .collect::<Vec<_>>()
                    .join("; ");
                callback((!header.is_empty()).then_some(header));
            },
        );
    }

    pub fn spin(&mut self) -> Vec<EngineEvent> {
        self.servo.spin_event_loop();
        self.shared.events.borrow_mut().drain(..).collect()
    }

    fn surface(&self, window: WindowId) -> Option<Rc<Surface>> {
        self.shared.surfaces.borrow().get(&window).cloned()
    }

    pub fn gl(&self, window: WindowId) -> Option<Arc<glow::Context>> {
        Some(self.surface(window)?.window_context.glow_gl_api())
    }

    pub fn make_current(&self, window: WindowId) {
        if let Some(surface) = self.surface(window)
            && let Err(e) = surface.window_context.make_current()
        {
            warn!("make_current failed: {e:?}");
        }
    }

    pub fn begin_frame(&self, window: WindowId) {
        let Some(surface) = self.surface(window) else {
            return;
        };
        if let Err(e) = surface.window_context.make_current() {
            warn!("make_current failed: {e:?}");
        }
        if let Some(webview) = self.visible_webview(window) {
            webview.paint();
        }
        surface.window_context.prepare_for_rendering();
    }

    pub fn present(&self, window: WindowId) {
        if let Some(surface) = self.surface(window) {
            surface.window_context.present();
        }
    }

    pub fn page_blitter(&self, window: WindowId) -> Option<PageBlitter> {
        let blit = self
            .surface(window)?
            .page_context
            .render_to_parent_callback()?;
        Some(Box::new(move |gl, [x, y, w, h]| {
            blit(
                gl,
                euclid::Rect::new(euclid::Point2D::new(x, y), euclid::Size2D::new(w, h)),
            )
        }))
    }

    pub fn resize_window(&self, window: WindowId, width: u32, height: u32) {
        if let Some(surface) = self.surface(window) {
            surface.window_context.resize(non_zero(width, height));
        }
    }

    pub fn resize_page(&self, window: WindowId, width: u32, height: u32) {
        let Some(surface) = self.surface(window) else {
            return;
        };
        let size = non_zero(width, height);
        if surface.page_context.size() == size {
            return;
        }

        match self.shared.webviews_of(window).first() {
            Some(webview) => webview.resize(size),
            None => surface.page_context.resize(size),
        }
    }

    pub fn set_scale(&self, window: WindowId, scale: f32) {
        let Some(surface) = self.surface(window) else {
            return;
        };
        surface.scale.set(scale);
        for webview in self.shared.webviews_of(window) {
            webview.set_hidpi_scale_factor(Scale::new(scale));
        }
    }

    pub fn set_page_focused(&self, window: WindowId, focused: bool) {
        if let Some(webview) = self.visible_webview(window) {
            webview.set_focused(focused);
        }
    }

    fn visible_webview(&self, window: WindowId) -> Option<WebView> {
        let tab = *self.shared.visible.borrow().get(&window)?;
        self.webview(tab)
    }

    fn webview(&self, tab: TabId) -> Option<WebView> {
        self.shared.webviews.borrow().get(&tab).cloned()
    }
}

impl Engine for ServoEngine {
    fn open_tab(&mut self, tab: TabId, window: WindowId, url: &Url, private: bool) {
        if private {
            warn!("private browsing is not isolated yet");
        }
        let Some(surface) = self.surface(window) else {
            log::error!("open_tab: no surface for {window}");
            return;
        };
        let rc: Rc<dyn RenderingContext> = surface.page_context.clone();
        let webview = WebViewBuilder::new(&self.servo, rc)
            .url(protocol::internal_url(url))
            .hidpi_scale_factor(Scale::new(surface.scale.get()))
            .delegate(self.delegate.clone())
            .build();
        self.shared.register(webview, window, tab);
    }

    fn close_tab(&mut self, tab: TabId) {
        self.shared.unregister(tab);
    }

    fn load(&mut self, tab: TabId, url: &Url) {
        if let Some(webview) = self.webview(tab) {
            webview.load(protocol::internal_url(url));
        }
    }

    fn go_back(&mut self, tab: TabId) {
        if let Some(webview) = self.webview(tab).filter(WebView::can_go_back) {
            webview.go_back(1);
        }
    }

    fn go_forward(&mut self, tab: TabId) {
        if let Some(webview) = self.webview(tab).filter(WebView::can_go_forward) {
            webview.go_forward(1);
        }
    }

    fn reload(&mut self, tab: TabId, bypass_cache: bool) {
        if bypass_cache {
            warn!("reload without cache is not supported by Servo yet; doing a normal reload");
        }
        if let Some(webview) = self.webview(tab) {
            webview.reload();
        }
    }

    fn stop(&mut self, tab: TabId) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        webview.evaluate_javascript("window.stop()", |result| {
            if let Err(error) = result {
                log::debug!("window.stop() failed: {error:?}");
            }
        });
    }

    fn set_zoom(&mut self, tab: TabId, factor: f32) {
        if let Some(webview) = self.webview(tab) {
            webview.set_page_zoom(factor);
        }
    }

    fn set_visible(&mut self, tab: TabId, visible: bool) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        let Some(window) = self.shared.window_of(tab) else {
            return;
        };
        let mut visible_tabs = self.shared.visible.borrow_mut();
        if visible {
            webview.show();
            visible_tabs.insert(window, tab);
        } else {
            webview.hide();
            if visible_tabs.get(&window) == Some(&tab) {
                visible_tabs.remove(&window);
            }
        }
    }

    fn send_input(&mut self, tab: TabId, input: InputEvent) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        webview.notify_input_event(convert::input_event(input));
    }

    fn find(&mut self, tab: TabId, request: FindRequest) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        let direction = match request.direction {
            FindDirection::First => "first",
            FindDirection::Next => "next",
            FindDirection::Previous => "previous",
            FindDirection::Clear => "clear",
        };

        let query = serde_json::to_string(&request.query).unwrap_or_else(|_| "\"\"".into());
        let script = format!(
            "({})({query}, {}, {}, \"{direction}\")",
            FIND_JS.trim().trim_end_matches(';'),
            request.case_sensitive,
            request.whole_word
        );
        let shared = self.shared.clone();
        webview.evaluate_javascript(script, move |result| {
            let result = match result {
                Ok(servo::JSValue::String(json)) => parse_find_result(&json),
                Ok(other) => {
                    log::debug!("find: unexpected result {other:?}");
                    None
                }
                Err(error) => {
                    log::debug!("find: {error:?}");
                    None
                }
            };
            if let Some(result) = result {
                shared
                    .events
                    .borrow_mut()
                    .push_back(EngineEvent::FindResult { tab, result });
            }
        });
    }

    fn check_readerable(&mut self, tab: TabId) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        let shared = self.shared.clone();
        let script = wrap_script(READERABLE_LIB_JS, READERABLE_JS);
        webview.evaluate_javascript(script, move |result| {
            let readerable = match result {
                Ok(servo::JSValue::Boolean(value)) => value,
                Ok(other) => {
                    log::debug!("readerable: unexpected result {other:?}");
                    false
                }
                Err(error) => {
                    log::debug!("readerable: {error:?}");
                    false
                }
            };
            shared
                .events
                .borrow_mut()
                .push_back(EngineEvent::Readerable { tab, readerable });
        });
    }

    fn show_reader(&mut self, tab: TabId) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        let Some(url) = webview.url() else {
            return;
        };
        let store = self.reader.clone();
        let target = webview.clone();
        let script = wrap_script(READABILITY_JS, READER_JS);
        webview.evaluate_javascript(script, move |result| {
            let article = match result {
                Ok(servo::JSValue::String(json)) => parse_article(&json, &url),
                Ok(other) => {
                    log::debug!("reader: unexpected result {other:?}");
                    None
                }
                Err(error) => {
                    log::debug!("reader: {error:?}");
                    None
                }
            };
            if let Some(article) = article {
                store.insert(article);
            }
            let reader = internal_url(&klin_core::reader::reader_url(&url));
            target.load(reader);
        });
    }

    fn answer(&mut self, request: RequestId, reply: Reply) {
        if let Some((tab, pending)) = self.shared.take(request) {
            log::debug!("answer {request}: {reply:?}");
            if matches!(
                reply,
                Reply::Menu(MenuAction::OpenLinkInNewTab | MenuAction::OpenImageInNewTab)
            ) {
                *self.shared.background_opener.borrow_mut() = Some(tab);
            }
            pending.answer(reply);
        }
    }
}

impl Drop for ServoEngine {
    fn drop(&mut self) {
        let pending: Vec<Pending> = self
            .shared
            .pending
            .borrow_mut()
            .drain()
            .map(|(_, (_, p))| p)
            .collect();
        for p in pending {
            p.answer(Reply::Dismiss);
        }
        self.shared.webviews.borrow_mut().clear();
    }
}
