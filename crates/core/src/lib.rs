//! Ядро Klin: состояние браузера и правила его изменения.
//!
//! Ядро не знает ни про egui, ни про Servo. С движком оно говорит через трейт [`Engine`]
//! и события [`EngineEvent`], с интерфейсом — через команды [`Command`].
//! Поведение воспроизводит Firefox; места, где мы сознательно отступаем, помечены
//! комментарием «Отступление от Firefox».

pub mod browser;
pub mod command;
pub mod downloads;
pub mod engine;
pub mod input;
pub mod prefs;
pub mod testing;
pub mod urifixup;
pub mod zoom;

pub use browser::{Browser, ClosedTab, Tab, VisitKind, Window};
pub use command::{Command, KeyChord, default_shortcuts};
pub use engine::{
    Engine, EngineEvent, MenuAction, MenuItem, PageRequest, PermissionFeature, Reply, RequestKind,
    SelectOption,
};
pub use input::InputEvent;
