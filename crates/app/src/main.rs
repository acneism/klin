//! Klin: точка входа и цикл событий winit.
//!
//! Одно окно: вкладки и адресная строка на egui, страница Servo под ними.
//! Горячие клавиши Firefox работают через команды ядра. Адрес можно передать аргументом:
//! `klin https://servo.org`.

use std::error::Error;
use std::sync::Arc;

use klin_core::input::{Key, Modifiers, MouseButton, NamedKey};
use klin_core::{
    Browser, Command, InputEvent, KeyChord, browser::UiEffect, command, default_shortcuts,
};
use klin_engine::{ServoEngine, Waker};
use klin_shared::{Url, WindowId};
use klin_ui::{Action, BrowserUi};
use log::{info, warn};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{self, KeyCode, ModifiersState, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowAttributes};

const APP_NAME: &str = "Klin";

#[derive(Debug)]
struct Wake;

fn main() -> Result<(), Box<dyn Error>> {
    // Логгер не ставим: его устанавливает `Servo::setup_logging` (env_logger, переменная RUST_LOG).
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| "failed to install rustls crypto provider")?;

    let start_url = std::env::args().nth(1).and_then(|arg| {
        klin_core::urifixup::to_url(&arg, &klin_core::urifixup::SearchEngine::default())
    });

    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let mut app = App::Starting { proxy, start_url };
    event_loop.run_app(&mut app)?;
    Ok(())
}

enum App {
    Starting {
        proxy: EventLoopProxy<Wake>,
        start_url: Option<Url>,
    },
    Running(Box<Running>),
    Failed,
}

struct Running {
    egui_state: egui_winit::State,
    painter: egui_glow::Painter,
    ui: BrowserUi,
    /// Область страницы в физических пикселях окна: x, y, ширина, высота.
    page_rect: [f32; 4],
    ui_has_focus: bool,
    // Порядок важен: движок отпускается раньше окна.
    engine: ServoEngine,
    window: Window,
    browser: Browser,
    browser_window: WindowId,
    shortcuts: Vec<(KeyChord, Command)>,
    modifiers: ModifiersState,
    cursor: (f32, f32),
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let App::Starting { proxy, start_url } = self else {
            return;
        };
        match Running::start(event_loop, proxy.clone(), start_url.take()) {
            Ok(running) => *self = App::Running(Box::new(running)),
            Err(error) => {
                log::error!("failed to start: {error}");
                *self = App::Failed;
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: Wake) {
        if let App::Running(app) = self {
            app.spin(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let App::Running(app) = self {
            app.window_event(event_loop, event);
            app.spin(event_loop);
        }
    }
}

impl Running {
    fn start(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Wake>,
        start_url: Option<Url>,
    ) -> Result<Self, Box<dyn Error>> {
        let window = event_loop.create_window(WindowAttributes::default().with_title(APP_NAME))?;
        let size = window.inner_size();
        let waker = Waker::new(move || {
            // Ошибка означает, что цикл событий уже завершён.
            let _ = proxy.send_event(Wake);
        });
        let mut engine = ServoEngine::new(
            event_loop.display_handle()?,
            window.window_handle()?,
            (size.width, size.height),
            window.scale_factor() as f32,
            waker,
        )?;
        let mut browser = Browser::new();
        let browser_window = browser.open_window(&mut engine, false, start_url.clone());

        let painter = egui_glow::Painter::new(engine.gl(), "", None, false)
            .map_err(|e| format!("egui painter: {e}"))?;
        let egui_state = egui_winit::State::new(
            egui::Context::default(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(painter.max_texture_side()),
        );
        let mut ui = BrowserUi::new();
        if start_url.is_none() {
            // Новое окно с пустой вкладкой: курсор сразу в адресной строке, как в Firefox.
            ui.focus_url_bar();
        }
        Ok(Self {
            egui_state,
            painter,
            ui,
            page_rect: [0.0; 4],
            ui_has_focus: false,
            engine,
            window,
            browser,
            browser_window,
            shortcuts: default_shortcuts(),
            modifiers: ModifiersState::empty(),
            cursor: (0.0, 0.0),
        })
    }

    fn spin(&mut self, event_loop: &ActiveEventLoop) {
        let events = self.engine.spin();
        if events.is_empty() {
            return;
        }
        for event in events {
            let effects = self.browser.handle_event(&mut self.engine, event);
            self.apply(event_loop, effects);
        }
        self.update_title();
        self.window.request_redraw();
    }

    /// Кадр: страница во внеэкранный буфер, затем интерфейс egui и страница в своей области.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.engine.begin_frame();
        let ctx = self.egui_state.egui_ctx().clone();
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let blitter: Option<Arc<klin_engine::PageBlitter>> =
            self.engine.page_blitter().map(Arc::new);
        let mut frame = None;
        let (ui, browser, window_id) = (&mut self.ui, &self.browser, self.browser_window);
        let full = ctx.run_ui(raw_input, |root| {
            let mut paint_page = |painter: &egui::Painter, rect: egui::Rect| {
                let Some(blit) = blitter.clone() else { return };
                let callback = egui_glow::CallbackFn::new(move |info, painter| {
                    let v = info.viewport_in_pixels();
                    blit(
                        painter.gl(),
                        [v.left_px, v.from_bottom_px, v.width_px, v.height_px],
                    );
                });
                painter.add(egui::PaintCallback {
                    rect,
                    callback: Arc::new(callback),
                });
            };
            frame = Some(ui.show(root, browser, window_id, &mut paint_page));
        });
        self.egui_state
            .handle_platform_output(&self.window, full.platform_output);
        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        let size = self.window.inner_size();
        let mut textures = full.textures_delta;
        self.painter.paint_and_update_textures(
            [size.width, size.height],
            full.pixels_per_point,
            &primitives,
            &mut textures,
        );
        self.engine.present();

        if let Some(frame) = frame {
            self.ui_has_focus = frame.ui_has_focus;
            let ppp = full.pixels_per_point;
            let r = frame.page_rect;
            self.page_rect = [
                r.min.x * ppp,
                r.min.y * ppp,
                r.width() * ppp,
                r.height() * ppp,
            ];
            self.engine.resize_page(
                self.page_rect[2].round() as u32,
                self.page_rect[3].round() as u32,
            );
            for action in frame.actions {
                self.act(event_loop, action);
            }
        }
        let repaint_now = full
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero());
        if repaint_now {
            self.window.request_redraw();
        }
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        match action {
            Action::Execute(cmd) => self.execute(event_loop, cmd),
            Action::SelectTab(tab) => self.browser.select_tab(&mut self.engine, tab),
            Action::CloseTab(tab) => {
                let effects = self.browser.close_tab(&mut self.engine, tab);
                self.apply(event_loop, effects);
            }
            Action::Navigate(input) => {
                self.browser
                    .load_input(&mut self.engine, self.browser_window, &input)
            }
            Action::FocusPage => self.engine.set_page_focused(true),
        }
        self.update_title();
        self.window.request_redraw();
    }

    fn execute(&mut self, event_loop: &ActiveEventLoop, cmd: Command) {
        let effects = self
            .browser
            .execute(&mut self.engine, self.browser_window, cmd);
        self.apply(event_loop, effects);
        self.window.request_redraw();
    }

    /// Точка окна → точка страницы, если она внутри страницы.
    fn page_point(&self, (x, y): (f32, f32)) -> Option<(f32, f32)> {
        let [px, py, w, h] = self.page_rect;
        let (lx, ly) = (x - px, y - py);
        (lx >= 0.0 && ly >= 0.0 && lx < w && ly < h).then_some((lx, ly))
    }

    fn apply(&mut self, event_loop: &ActiveEventLoop, effects: Vec<UiEffect>) {
        for effect in effects {
            match effect {
                UiEffect::Redraw(_) => self.window.request_redraw(),
                UiEffect::WindowClosed(id) if id == self.browser_window => event_loop.exit(),
                UiEffect::WindowOpened(_) => {
                    warn!("multiple windows are not supported in the prototype yet")
                }
                UiEffect::FocusUrlBar(_) => {
                    self.engine.set_page_focused(false);
                    self.ui.focus_url_bar();
                    self.window.request_redraw();
                }
                UiEffect::ShowRequest { id, kind, .. } => {
                    // До интерфейса диалогов отвечаем отказом, как при закрытии диалога.
                    info!("page request {kind:?} dismissed: no dialog UI yet");
                    klin_core::Engine::answer(&mut self.engine, id, klin_core::Reply::Dismiss);
                }
                UiEffect::NotImplemented(cmd) => info!("{cmd:?} is planned for a later version"),
                UiEffect::WindowClosed(_) => {}
            }
        }
    }

    fn update_title(&self) {
        let title = self
            .browser
            .selected_tab(self.browser_window)
            .map(|t| t.title.trim().to_owned())
            .filter(|t| !t.is_empty());
        // Как в Firefox: «Заголовок страницы — Имя браузера».
        self.window.set_title(&match title {
            Some(t) => format!("{t} — {APP_NAME}"),
            None => APP_NAME.to_owned(),
        });
    }

    fn selected_tab(&self) -> Option<klin_shared::TabId> {
        self.browser.window(self.browser_window)?.selected_tab()
    }

    fn send(&mut self, input: InputEvent) {
        if let Some(tab) = self.selected_tab() {
            klin_core::Engine::send_input(&mut self.engine, tab, input);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop);
            return;
        }
        let egui_response = self.egui_state.on_window_event(&self.window, &event);
        if egui_response.repaint {
            self.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.engine.resize_window(size.width, size.height);
                self.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.engine.set_scale(scale_factor as f32)
            }
            WindowEvent::Focused(focused) => {
                self.engine.set_page_focused(focused && !self.ui_has_focus)
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let modifiers = modifiers(self.modifiers);
                let pressed = event.state == ElementState::Pressed;
                let key = key(&event.logical_key, event.physical_key, modifiers);
                if pressed {
                    let cmd = key.and_then(|k| {
                        command::lookup(&self.shortcuts, KeyChord::new(modifiers, k))
                    });
                    // Esc в адресной строке отменяет правку, а не останавливает загрузку.
                    let ui_owns = self.ui_has_focus && cmd == Some(Command::Stop);
                    if let Some(cmd) = cmd.filter(|_| !ui_owns) {
                        self.execute(event_loop, cmd);
                        return;
                    }
                }
                if self.ui_has_focus {
                    return; // клавиша уже у egui
                }
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => Some(format!("{code:?}")),
                    PhysicalKey::Unidentified(_) => None,
                };
                self.send(InputEvent::Key {
                    key: key.unwrap_or(Key::Named(NamedKey::Other)),
                    text: event.text.map(|t| t.to_string()),
                    code,
                    modifiers,
                    pressed,
                    repeat: event.repeat,
                });
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                if let Some((x, y)) = self.page_point(self.cursor) {
                    self.send(InputEvent::MouseMove { x, y });
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Back => MouseButton::Back,
                    winit::event::MouseButton::Forward => MouseButton::Forward,
                    winit::event::MouseButton::Other(_) => return,
                };
                let pressed = state == ElementState::Pressed;
                // Боковые кнопки мыши — «Назад»/«Вперёд», как в Firefox.
                if pressed && matches!(button, MouseButton::Back | MouseButton::Forward) {
                    let cmd = if button == MouseButton::Back {
                        Command::Back
                    } else {
                        Command::Forward
                    };
                    self.execute(event_loop, cmd);
                    return;
                }
                if let Some((x, y)) = self.page_point(self.cursor) {
                    self.send(InputEvent::MouseButton {
                        button,
                        x,
                        y,
                        pressed,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // 76 пикселей на строку — как в servoshell.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 76.0, f64::from(y) * 76.0),
                    MouseScrollDelta::PixelDelta(d) => (d.x, d.y),
                };
                if let Some((x, y)) = self.page_point(self.cursor) {
                    self.send(InputEvent::Wheel { dx, dy, x, y });
                }
            }
            _ => {}
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Текстуры egui удаляются, пока GL-контекст окна ещё жив.
        self.painter.destroy();
    }
}

fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        ctrl: state.control_key(),
        shift: state.shift_key(),
        alt: state.alt_key(),
        meta: state.super_key(),
    }
}

/// Клавиша для таблицы горячих клавиш. С Ctrl/Alt буквы и цифры берутся по физической
/// клавише, чтобы сочетания работали в любой раскладке (в Firefox Ctrl+T работает и в русской).
fn key(logical: &keyboard::Key, physical: PhysicalKey, mods: Modifiers) -> Option<Key> {
    use keyboard::NamedKey as W;
    if (mods.ctrl || mods.alt)
        && let PhysicalKey::Code(code) = physical
        && let Some(c) = latin_from_code(code)
    {
        return Some(Key::Char(c));
    }
    Some(match logical {
        keyboard::Key::Character(s) => Key::Char(s.chars().next()?.to_ascii_lowercase()),
        keyboard::Key::Named(named) => Key::Named(match named {
            W::Enter => NamedKey::Enter,
            W::Escape => NamedKey::Escape,
            W::Tab => NamedKey::Tab,
            W::Backspace => NamedKey::Backspace,
            W::Delete => NamedKey::Delete,
            W::Home => NamedKey::Home,
            W::End => NamedKey::End,
            W::PageUp => NamedKey::PageUp,
            W::PageDown => NamedKey::PageDown,
            W::ArrowLeft => NamedKey::ArrowLeft,
            W::ArrowRight => NamedKey::ArrowRight,
            W::ArrowUp => NamedKey::ArrowUp,
            W::ArrowDown => NamedKey::ArrowDown,
            W::F1 => NamedKey::F(1),
            W::F2 => NamedKey::F(2),
            W::F3 => NamedKey::F(3),
            W::F4 => NamedKey::F(4),
            W::F5 => NamedKey::F(5),
            W::F6 => NamedKey::F(6),
            W::F7 => NamedKey::F(7),
            W::F8 => NamedKey::F(8),
            W::F9 => NamedKey::F(9),
            W::F10 => NamedKey::F(10),
            W::F11 => NamedKey::F(11),
            W::F12 => NamedKey::F(12),
            W::Space => return Some(Key::Char(' ')),
            _ => NamedKey::Other,
        }),
        _ => return None,
    })
}

fn latin_from_code(code: KeyCode) -> Option<char> {
    let name = format!("{code:?}");
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.chars().next().map(|c| c.to_ascii_lowercase());
    }
    if let Some(digit) = name
        .strip_prefix("Digit")
        .or_else(|| name.strip_prefix("Numpad"))
        && digit.len() == 1
    {
        return digit.chars().next();
    }
    match code {
        KeyCode::Equal => Some('='),
        KeyCode::Minus | KeyCode::NumpadSubtract => Some('-'),
        KeyCode::NumpadAdd => Some('+'),
        _ => None,
    }
}
