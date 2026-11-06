//! Хранилища профиля. Каждое работает в своём потоке и общается с остальным браузером
//! сообщениями: поток интерфейса не ходит на диск (docs/spec.md, «Отзывчивость»).
//!
//! Готово: [`content_prefs`] (настройки по сайтам, в том числе масштаб).
//! Дальше (0.2): places, logins, sessionstore.

pub mod content_prefs;

pub use content_prefs::ContentPrefs;
