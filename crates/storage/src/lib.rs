//! Хранилища профиля. Каждое работает в своём потоке и общается с остальным браузером
//! сообщениями: поток интерфейса не ходит на диск (docs/spec.md, «Отзывчивость»).
//!
//! Готово: [`content_prefs`] (настройки по сайтам, в том числе масштаб), [`places`]
//! (история и закладки, перенос из application-services) и служба [`places_service`].
//! [`sessionstore`] — восстановление сессии в формате Firefox. Дальше: logins (0.3).

pub mod content_prefs;
pub mod places;
pub mod places_service;
pub mod sessionstore;
pub(crate) mod support;

pub use content_prefs::ContentPrefs;
pub use places_service::PlacesService;
