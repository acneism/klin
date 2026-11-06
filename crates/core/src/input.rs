//! События ввода, которые интерфейс передаёт странице.
//! Свои типы, а не типы Servo: ядро не зависит от движка.

/// Модификаторы клавиатуры.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };
    pub const CTRL_SHIFT: Self = Self {
        ctrl: true,
        shift: true,
        ..Self::NONE
    };
    pub const CTRL_ALT: Self = Self {
        ctrl: true,
        alt: true,
        ..Self::NONE
    };
}

/// Клавиша без учёта раскладки там, где Firefox её не учитывает.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// Символьная клавиша; буквы хранятся в нижнем регистре латиницы.
    Char(char),
    Named(NamedKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    F(u8),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Key {
        key: Key,
        /// Текст, который вводит клавиша с учётом раскладки.
        text: Option<String>,
        /// Физическая клавиша, имя по W3C UI Events (`KeyA`, `Digit1`).
        code: Option<String>,
        modifiers: Modifiers,
        pressed: bool,
        repeat: bool,
    },
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseButton {
        button: MouseButton,
        x: f32,
        y: f32,
        pressed: bool,
    },
    /// Прокрутка в пикселях устройства.
    Wheel {
        dx: f64,
        dy: f64,
        x: f32,
        y: f32,
    },
}
