use std::str::FromStr;

use klin_core::InputEvent;
use klin_core::input::{Key, MouseButton, NamedKey};
use klin_shared::Image;
use servo::{DevicePoint, PixelFormat};
use servo::{
    KeyState, KeyboardEvent, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, WheelDelta,
    WheelEvent, WheelMode,
};

pub(crate) fn input_event(input: InputEvent) -> servo::InputEvent {
    let point = |x: f32, y: f32| DevicePoint::new(x, y).into();

    match input {
        InputEvent::Key {
            key,
            text,
            code,
            modifiers,
            pressed,
            repeat,
        } => {
            let state = if pressed {
                KeyState::Down
            } else {
                KeyState::Up
            };
            let key = match (text, key) {
                (Some(text), _) if !text.is_empty() => keyboard_types::Key::Character(text),
                (_, Key::Char(c)) => keyboard_types::Key::Character(c.to_string()),
                (_, Key::Named(named)) => keyboard_types::Key::Named(named_key(named)),
            };
            let code = code
                .and_then(|c| keyboard_types::Code::from_str(&c).ok())
                .unwrap_or(keyboard_types::Code::Unidentified);
            let mut mods = keyboard_types::Modifiers::empty();
            mods.set(keyboard_types::Modifiers::CONTROL, modifiers.ctrl);
            mods.set(keyboard_types::Modifiers::SHIFT, modifiers.shift);
            mods.set(keyboard_types::Modifiers::ALT, modifiers.alt);
            mods.set(keyboard_types::Modifiers::META, modifiers.meta);
            servo::InputEvent::Keyboard(KeyboardEvent::new(keyboard_types::KeyboardEvent {
                state,
                key,
                code,
                location: keyboard_types::Location::Standard,
                modifiers: mods,
                repeat,
                is_composing: false,
            }))
        }
        InputEvent::MouseMove { x, y } => {
            servo::InputEvent::MouseMove(MouseMoveEvent::new(point(x, y)))
        }
        InputEvent::MouseButton {
            button,
            x,
            y,
            pressed,
        } => {
            let action = if pressed {
                MouseButtonAction::Down
            } else {
                MouseButtonAction::Up
            };
            let button = match button {
                MouseButton::Left => servo::MouseButton::Primary,
                MouseButton::Middle => servo::MouseButton::Auxiliary,
                MouseButton::Right => servo::MouseButton::Secondary,
                MouseButton::Back => servo::MouseButton::Back,
                MouseButton::Forward => servo::MouseButton::Forward,
            };
            servo::InputEvent::MouseButton(MouseButtonEvent::new(action, button, point(x, y)))
        }
        InputEvent::Wheel { dx, dy, x, y } => servo::InputEvent::Wheel(WheelEvent::new(
            WheelDelta {
                x: dx,
                y: dy,
                z: 0.0,
                mode: WheelMode::DeltaPixel,
            },
            point(x, y),
        )),
    }
}

fn named_key(key: NamedKey) -> keyboard_types::NamedKey {
    use keyboard_types::NamedKey as K;
    match key {
        NamedKey::Enter => K::Enter,
        NamedKey::Escape => K::Escape,
        NamedKey::Tab => K::Tab,
        NamedKey::Backspace => K::Backspace,
        NamedKey::Delete => K::Delete,
        NamedKey::Home => K::Home,
        NamedKey::End => K::End,
        NamedKey::PageUp => K::PageUp,
        NamedKey::PageDown => K::PageDown,
        NamedKey::ArrowLeft => K::ArrowLeft,
        NamedKey::ArrowRight => K::ArrowRight,
        NamedKey::ArrowUp => K::ArrowUp,
        NamedKey::ArrowDown => K::ArrowDown,
        NamedKey::F(n) => K::from_str(&format!("F{n}")).unwrap_or(K::Unidentified),
        NamedKey::Other => K::Unidentified,
    }
}

pub(crate) fn image(image: &servo::Image) -> Option<Image> {
    let data = image.data();
    let rgba = match image.format {
        PixelFormat::RGBA8 => data.to_vec(),
        PixelFormat::BGRA8 => data
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect(),
        PixelFormat::RGB8 => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        PixelFormat::K8 => data.iter().flat_map(|&k| [k, k, k, 255]).collect(),
        PixelFormat::KA8 => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
    };
    Some(Image {
        width: image.width,
        height: image.height,
        rgba,
    })
}
