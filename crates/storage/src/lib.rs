//! Хранилища профиля. Каждое работает в своём потоке и общается с остальным браузером
//! сообщениями: поток интерфейса не ходит на диск (docs/spec.md, «Отзывчивость»).
//!
//! Готово: [`content_prefs`] (настройки по сайтам, в том числе масштаб), [`places`]
//! (история и закладки, перенос из application-services) и служба [`places_service`].
//! Дальше (0.2): sessionstore; logins — 0.3.

pub mod content_prefs;
pub mod places;
pub mod places_service;
pub(crate) mod support;

pub use content_prefs::ContentPrefs;
pub use places_service::PlacesService;
