//! Общие типы Klin: идентификаторы окон, вкладок и запросов.
//!
//! Крейт не зависит ни от движка, ни от интерфейса — его используют все остальные.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

pub use url::Url;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u64);

        impl $name {
            /// Выдаёт новый уникальный в пределах процесса идентификатор.
            pub fn next() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                Self(NEXT.fetch_add(1, Ordering::Relaxed))
            }

            pub fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(
    /// Окно браузера.
    WindowId,
    "window#"
);
id_type!(
    /// Вкладка. Живёт дольше, чем WebView движка: выгруженная вкладка сохраняет TabId.
    TabId,
    "tab#"
);
id_type!(
    /// Запрос страницы к встраивателю (диалог, разрешение, выбор файла, меню).
    RequestId,
    "request#"
);

/// Состояние загрузки страницы — как индикатор во вкладке Firefox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    /// Загрузка началась, документа ещё нет.
    Started,
    /// Документ разобран (DOMContentLoaded).
    HeadParsed,
    /// Страница загружена полностью (load).
    #[default]
    Complete,
}

/// Куда открыть новую вкладку, запрошенную страницей (window.open, target=_blank, средняя кнопка).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    ForegroundTab,
    BackgroundTab,
    NewWindow,
}

/// Изображение в формате RGBA8 (значок сайта, снимок вкладки).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = TabId::next();
        let b = TabId::next();
        assert_ne!(a, b);
        assert!(b > a);
        assert!(a.to_string().starts_with("tab#"));
    }
}
