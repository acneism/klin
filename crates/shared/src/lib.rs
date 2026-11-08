use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

pub use url::Url;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u64);

        impl $name {
            pub fn next() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                Self(NEXT.fetch_add(1, Ordering::Relaxed))
            }

            pub fn get(self) -> u64 {
                self.0
            }

            pub fn from_raw(value: u64) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(WindowId, "window#");
id_type!(TabId, "tab#");
id_type!(RequestId, "request#");
id_type!(DownloadId, "download#");

pub mod addons;
pub mod bookmarks;
pub mod download;
pub mod import;
pub mod logins;
pub mod session;
pub mod tracking;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    Started,
    HeadParsed,
    #[default]
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    ForegroundTab,
    BackgroundTab,
    NewWindow,
}

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
