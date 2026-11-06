//! Klin: точка входа и цикл событий winit.
//!
//! Этап 0 (прототип): одно окно, страница на всю клиентскую область, горячие клавиши
//! Firefox работают через команды ядра. Интерфейс egui (вкладки, адресная строка)
//! подключается следующим шагом; пока адрес можно передать аргументом:
//! `klin https://servo.org`.

use std::error::Error;

use klin_core::input::{Key, Modifiers, MouseButton, NamedKey};
use klin_core::{
    Browser, Command, InputEvent, KeyChord, browser::UiEffect, command, default_shortcuts,
};
use klin_engine::{ServoEngine, Waker};
use klin_shared::{Url, WindowId};
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
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
        let browser_window = browser.open_window(&mut engine, false, start_url);
        Ok(Self {
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
        for event in self.engine.spin() {
            let effects = self.browser.handle_event(&mut self.engine, event);
            self.apply(event_loop, effects);
        }
        self.update_title();
    }

    fn apply(&mut self, event_loop: &ActiveEventLoop, effects: Vec<UiEffect>) {
        for effect in effects {
            match effect {
                UiEffect::Redraw(_) => self.window.request_redraw(),
                UiEffect::WindowClosed(id) if id == self.browser_window => event_loop.exit(),
                UiEffect::WindowOpened(_) => {
                    warn!("multiple windows are not supported in the prototype yet")
                }
                UiEffect::FocusUrlBar(_) => info!("address bar arrives with the egui interface"),
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.engine.paint(),
            WindowEvent::Resized(size) => self.engine.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.engine.set_scale(scale_factor as f32)
            }
            WindowEvent::Focused(focused) => self.engine.set_page_focused(focused),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let modifiers = modifiers(self.modifiers);
                let pressed = event.state == ElementState::Pressed;
                let key = key(&event.logical_key, event.physical_key, modifiers);
                if pressed
                    && let Some(cmd) = key
                        .and_then(|k| command::lookup(&self.shortcuts, KeyChord::new(modifiers, k)))
                {
                    let effects = self
                        .browser
                        .execute(&mut self.engine, self.browser_window, cmd);
                    self.apply(event_loop, effects);
                    return;
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
                self.send(InputEvent::MouseMove {
                    x: self.cursor.0,
                    y: self.cursor.1,
                });
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
                    let effects = self
                        .browser
                        .execute(&mut self.engine, self.browser_window, cmd);
                    self.apply(event_loop, effects);
                    return;
                }
                let (x, y) = self.cursor;
                self.send(InputEvent::MouseButton {
                    button,
                    x,
                    y,
                    pressed,
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // 76 пикселей на строку — как в servoshell.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 76.0, f64::from(y) * 76.0),
                    MouseScrollDelta::PixelDelta(d) => (d.x, d.y),
                };
                let (x, y) = self.cursor;
                self.send(InputEvent::Wheel { dx, dy, x, y });
            }
            _ => {}
        }
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
