//! Масштаб страницы — как ZoomManager в Firefox.

use std::collections::HashMap;

/// `toolkit.zoomManager.zoomValues`
pub const ZOOM_VALUES: [f32; 16] = [
    0.3, 0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.2, 1.33, 1.5, 1.7, 2.0, 2.4, 3.0, 4.0, 5.0,
];

/// Следующее значение масштаба (Ctrl+ +).
pub fn enlarge(current: f32) -> f32 {
    ZOOM_VALUES
        .iter()
        .copied()
        .find(|&z| z > current + 0.001)
        .unwrap_or(current)
}

/// Предыдущее значение масштаба (Ctrl+ −).
pub fn reduce(current: f32) -> f32 {
    ZOOM_VALUES
        .iter()
        .rev()
        .copied()
        .find(|&z| z < current - 0.001)
        .unwrap_or(current)
}

/// Масштаб по сайтам (`browser.zoom.siteSpecific`). В Firefox хранится в content-prefs.sqlite;
/// здесь пока в памяти, запись на диск — вместе с профилем в 0.2.
#[derive(Debug, Default)]
pub struct SiteZoom {
    by_host: HashMap<String, f32>,
}

impl SiteZoom {
    pub fn get(&self, host: &str) -> f32 {
        self.by_host.get(host).copied().unwrap_or(1.0)
    }

    pub fn set(&mut self, host: &str, zoom: f32) {
        if (zoom - 1.0).abs() < 0.001 {
            self.by_host.remove(host);
        } else {
            self.by_host.insert(host.to_owned(), zoom);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_firefox_values() {
        assert_eq!(enlarge(1.0), 1.1);
        assert_eq!(reduce(1.0), 0.9);
        assert_eq!(enlarge(5.0), 5.0);
        assert_eq!(reduce(0.3), 0.3);
        assert_eq!(enlarge(1.05), 1.1);
    }
}
