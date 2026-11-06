//! Адаптер движка Servo — единственный крейт Klin, который зависит от `servo`.
//!
//! Когда API Servo меняется (ежемесячные выпуски могут его ломать), правится только
//! этот крейт. Наружу он отдаёт реализацию [`klin_core::Engine`] и методы для цикла событий
//! и отрисовки.
//!
//! Отрисовка — как в servoshell: страницы рисуются во внеэкранный буфер
//! (`OffscreenRenderingContext`), интерфейс egui рисуется в окно через тот же GL-контекст,
//! а страница копируется в свою область окна функцией из [`ServoEngine::page_blitter`].

mod convert;
mod delegate;
mod protocol;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use euclid::Scale;
use klin_core::{Engine, EngineEvent, InputEvent, Reply};
use klin_shared::{RequestId, TabId, Url, WindowId};
use log::warn;
use raw_window_handle::{DisplayHandle, WindowHandle};
use servo::{
    EventLoopWaker, OffscreenRenderingContext, RenderingContext, Servo, ServoBuilder, WebView,
    WebViewBuilder, WindowRenderingContext,
};

use crate::delegate::{Delegate, Pending, Shared};

pub use crate::protocol::internal_url;

/// Пробуждение цикла событий из любого потока: Servo зовёт его, когда у движка есть работа.
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

/// См. [`ServoEngine::page_blitter`].
pub type PageBlitter = Box<dyn Fn(&glow::Context, [i32; 4]) + Send + Sync>;

/// Servo не принимает буферы нулевого размера (свёрнутое окно).
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

/// Движок Servo для одного окна.
///
/// TODO(этап 0): несколько окон — по `RenderingContext` на окно; сейчас все вкладки
/// рисуются в контекст первого окна.
pub struct ServoEngine {
    servo: Servo,
    window_context: Rc<WindowRenderingContext>,
    /// Сюда рисуют все WebView; размер — область страницы в окне.
    page_context: Rc<OffscreenRenderingContext>,
    shared: Rc<Shared>,
    delegate: Rc<Delegate>,
    scale: f32,
}

impl ServoEngine {
    /// Запустить Servo и привязать его к окну.
    pub fn new(
        display: DisplayHandle<'_>,
        window: WindowHandle<'_>,
        size: (u32, u32),
        scale: f32,
        waker: Waker,
    ) -> Result<Self, EngineError> {
        let window_context = Rc::new(
            WindowRenderingContext::new(display, window, dpi::PhysicalSize::new(size.0, size.1))
                .map_err(|e| EngineError(format!("rendering context: {e:?}")))?,
        );
        window_context
            .make_current()
            .map_err(|e| EngineError(format!("make_current: {e:?}")))?;

        let mut protocols = servo::protocol_handler::ProtocolRegistry::with_internal_protocols();
        protocols
            .register(klin_internal_pages::SCHEME, protocol::InternalPages)
            .map_err(|e| EngineError(format!("register {}: {e:?}", klin_internal_pages::SCHEME)))?;

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker))
            .protocol_registry(protocols)
            .build();
        servo.setup_logging();

        let shared = Rc::new(Shared {
            events: RefCell::new(VecDeque::new()),
            tabs: RefCell::new(HashMap::new()),
            webviews: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashMap::new()),
            visible: RefCell::new(None),
        });
        let page_context = Rc::new(window_context.offscreen_context(non_zero(size.0, size.1)));
        let rc: Rc<dyn RenderingContext> = page_context.clone();
        let delegate = Delegate::new(shared.clone(), rc, scale);

        Ok(Self {
            servo,
            window_context,
            page_context,
            shared,
            delegate,
            scale,
        })
    }

    /// Дать движку поработать и забрать накопившиеся события.
    pub fn spin(&mut self) -> Vec<EngineEvent> {
        self.servo.spin_event_loop();
        self.shared.events.borrow_mut().drain(..).collect()
    }

    /// GL-контекст окна для отрисовки интерфейса (egui_glow).
    pub fn gl(&self) -> Arc<glow::Context> {
        self.window_context.glow_gl_api()
    }

    /// Начать кадр: нарисовать видимую страницу во внеэкранный буфер и подготовить окно.
    pub fn begin_frame(&self) {
        if let Err(e) = self.window_context.make_current() {
            warn!("make_current failed: {e:?}");
        }
        if let Some(webview) = self.visible_webview() {
            webview.paint();
        }
        self.window_context.prepare_for_rendering();
    }

    /// Показать готовый кадр окна.
    pub fn present(&self) {
        self.window_context.present();
    }

    /// Функция, копирующая страницу в прямоугольник окна `[x, y, ширина, высота]`
    /// в пикселях устройства, начало координат — внизу слева (как в OpenGL).
    pub fn page_blitter(&self) -> Option<PageBlitter> {
        let blit = self.page_context.render_to_parent_callback()?;
        Some(Box::new(move |gl, [x, y, w, h]| {
            blit(
                gl,
                euclid::Rect::new(euclid::Point2D::new(x, y), euclid::Size2D::new(w, h)),
            )
        }))
    }

    /// Размер окна изменился.
    pub fn resize_window(&self, width: u32, height: u32) {
        self.window_context.resize(non_zero(width, height));
    }

    /// Размер области страницы изменился (окно минус панели интерфейса).
    pub fn resize_page(&self, width: u32, height: u32) {
        let size = non_zero(width, height);
        if self.page_context.size() == size {
            return;
        }
        self.page_context.resize(size);
        for webview in self.shared.webviews.borrow().values() {
            webview.resize(size);
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.delegate.set_scale(scale);
        for webview in self.shared.webviews.borrow().values() {
            webview.set_hidpi_scale_factor(Scale::new(scale));
        }
    }

    /// Передать фокус ввода странице (или забрать, когда фокус в интерфейсе).
    pub fn set_page_focused(&self, focused: bool) {
        if let Some(webview) = self.visible_webview() {
            webview.set_focused(focused);
        }
    }

    fn visible_webview(&self) -> Option<WebView> {
        let tab = (*self.shared.visible.borrow())?;
        self.webview(tab)
    }

    fn webview(&self, tab: TabId) -> Option<WebView> {
        self.shared.webviews.borrow().get(&tab).cloned()
    }
}

impl Engine for ServoEngine {
    fn open_tab(&mut self, _window: WindowId, url: &Url, private: bool) -> TabId {
        if private {
            // Отдельного контекста хранилища в API Servo пока не нашли — проверить до 0.2.
            warn!("private browsing is not isolated yet");
        }
        let rc: Rc<dyn RenderingContext> = self.page_context.clone();
        let webview = WebViewBuilder::new(&self.servo, rc)
            .url(protocol::internal_url(url))
            .hidpi_scale_factor(Scale::new(self.scale))
            .delegate(self.delegate.clone())
            .build();
        self.shared.register(webview)
    }

    fn close_tab(&mut self, tab: TabId) {
        // WebView закрывается, когда отпущена последняя ссылка на неё.
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
            // В API Servo нет перезагрузки в обход кэша — кандидат на патч.
            warn!("reload without cache is not supported by Servo yet; doing a normal reload");
        }
        if let Some(webview) = self.webview(tab) {
            webview.reload();
        }
    }

    fn stop(&mut self, _tab: TabId) {
        // В API WebView нет остановки загрузки — патч в Servo (docs/spec.md).
        warn!("stop is not supported by Servo yet");
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
        if visible {
            webview.show();
            *self.shared.visible.borrow_mut() = Some(tab);
        } else {
            webview.hide();
            let mut current = self.shared.visible.borrow_mut();
            if *current == Some(tab) {
                *current = None;
            }
        }
    }

    fn send_input(&mut self, tab: TabId, input: InputEvent) {
        let Some(webview) = self.webview(tab) else {
            return;
        };
        webview.notify_input_event(convert::input_event(input));
    }

    fn answer(&mut self, request: RequestId, reply: Reply) {
        let Some(pending) = self.shared.pending.borrow_mut().remove(&request) else {
            return;
        };
        pending.answer(reply);
    }
}

impl Drop for ServoEngine {
    fn drop(&mut self) {
        // Ответить «нет» на висящие запросы до остановки движка.
        let pending: Vec<Pending> = self
            .shared
            .pending
            .borrow_mut()
            .drain()
            .map(|(_, p)| p)
            .collect();
        for p in pending {
            p.answer(Reply::Dismiss);
        }
        self.shared.webviews.borrow_mut().clear();
    }
}
