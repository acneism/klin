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

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use euclid::Scale;
use klin_core::{Engine, EngineEvent, InputEvent, MenuAction, Reply};
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

/// Где движок хранит данные профиля.
#[derive(Clone, Debug, Default)]
pub struct EngineConfig {
    /// Папка профиля: cookies, localStorage, HSTS и другие данные Servo.
    pub data_dir: Option<std::path::PathBuf>,
    /// Файл дискового HTTP-кэша (в папке кэша профиля).
    pub http_cache: Option<std::path::PathBuf>,
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

/// Поверхность окна: GL-контекст окна и внеэкранный буфер, куда рисуют его вкладки.
pub(crate) struct Surface {
    window_context: Rc<WindowRenderingContext>,
    /// Сюда рисуют WebView этого окна; размер — область страницы в окне.
    pub page_context: Rc<OffscreenRenderingContext>,
    pub scale: Cell<f32>,
}

/// Движок Servo: один на процесс, поверхность — на каждое окно.
pub struct ServoEngine {
    servo: Servo,
    shared: Rc<Shared>,
    delegate: Rc<Delegate>,
}

impl ServoEngine {
    /// Запустить Servo. Окна добавляются через [`ServoEngine::add_window`].
    pub fn new(waker: Waker, config: EngineConfig) -> Result<Self, EngineError> {
        let mut protocols = servo::protocol_handler::ProtocolRegistry::with_internal_protocols();
        protocols
            .register(klin_internal_pages::SCHEME, protocol::InternalPages)
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
        })
    }

    /// Подключить окно ОС: вкладки окна `window` будут рисоваться в него.
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

    /// Окно закрыто. Его вкладки к этому моменту уже закрыты ядром.
    pub fn remove_window(&mut self, window: WindowId) {
        self.shared.surfaces.borrow_mut().remove(&window);
        self.shared.visible.borrow_mut().remove(&window);
    }

    /// User-Agent движка: служба загрузок представляется сайтам так же, как страницы.
    pub fn user_agent(&self) -> String {
        servo::pref!(user_agent)
    }

    /// Заголовок `Cookie` для адреса из хранилища движка (включая HttpOnly);
    /// `callback` вызывается в главном потоке.
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

    /// Дать движку поработать и забрать накопившиеся события.
    pub fn spin(&mut self) -> Vec<EngineEvent> {
        self.servo.spin_event_loop();
        self.shared.events.borrow_mut().drain(..).collect()
    }

    fn surface(&self, window: WindowId) -> Option<Rc<Surface>> {
        self.shared.surfaces.borrow().get(&window).cloned()
    }

    /// GL-контекст окна для отрисовки интерфейса (egui_glow).
    pub fn gl(&self, window: WindowId) -> Option<Arc<glow::Context>> {
        Some(self.surface(window)?.window_context.glow_gl_api())
    }

    /// Сделать GL-контекст окна текущим (например, чтобы освободить ресурсы интерфейса).
    pub fn make_current(&self, window: WindowId) {
        if let Some(surface) = self.surface(window)
            && let Err(e) = surface.window_context.make_current()
        {
            warn!("make_current failed: {e:?}");
        }
    }

    /// Начать кадр окна: нарисовать видимую страницу во внеэкранный буфер и подготовить окно.
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

    /// Показать готовый кадр окна.
    pub fn present(&self, window: WindowId) {
        if let Some(surface) = self.surface(window) {
            surface.window_context.present();
        }
    }

    /// Функция, копирующая страницу окна в прямоугольник `[x, y, ширина, высота]`
    /// в пикселях устройства, начало координат — внизу слева (как в OpenGL).
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

    /// Размер окна изменился.
    pub fn resize_window(&self, window: WindowId, width: u32, height: u32) {
        if let Some(surface) = self.surface(window) {
            surface.window_context.resize(non_zero(width, height));
        }
    }

    /// Размер области страницы изменился (окно минус панели интерфейса).
    pub fn resize_page(&self, window: WindowId, width: u32, height: u32) {
        let Some(surface) = self.surface(window) else {
            return;
        };
        let size = non_zero(width, height);
        if surface.page_context.size() == size {
            return;
        }
        surface.page_context.resize(size);
        for webview in self.shared.webviews_of(window) {
            webview.resize(size);
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

    /// Передать фокус ввода странице окна (или забрать, когда фокус в интерфейсе).
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
    fn open_tab(&mut self, window: WindowId, url: &Url, private: bool) -> TabId {
        if private {
            // Отдельного контекста хранилища в API Servo пока не нашли — проверить до 0.2.
            warn!("private browsing is not isolated yet");
        }
        let Some(surface) = self.surface(window) else {
            // Ядро открывает вкладки только в окнах, для которых вызван add_window.
            log::error!("open_tab: no surface for {window}");
            return TabId::next();
        };
        let rc: Rc<dyn RenderingContext> = surface.page_context.clone();
        let webview = WebViewBuilder::new(&self.servo, rc)
            .url(protocol::internal_url(url))
            .hidpi_scale_factor(Scale::new(surface.scale.get()))
            .delegate(self.delegate.clone())
            .build();
        self.shared.register(webview, window)
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

    fn stop(&mut self, tab: TabId) {
        // В API WebView нет остановки загрузки, а патчи в Servo мы не отправляем: правила
        // Servo запрещают код, написанный ИИ. Поэтому — window.stop() в странице.
        // Отступление от Firefox: window.stop() останавливает загрузку документа и его
        // фреймов, но не отменяет переход, который ещё не дошёл до нового документа.
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
        // Ответить «нет» на висящие запросы до остановки движка.
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
