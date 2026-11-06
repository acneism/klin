//! Настройки браузера. Имена и значения по умолчанию — из Firefox (about:config).
//! Формат prefs.js будет перенесён в 0.2 вместе с about:preferences.

/// `browser.startup.homepage`
pub const HOMEPAGE: &str = "about:home";
/// Адрес новой вкладки (`browser.newtab.url` в старых версиях Firefox; сейчас — about:newtab).
pub const NEW_TAB_URL: &str = "about:newtab";
/// `browser.sessionstore.max_tabs_undo`
pub const MAX_TABS_UNDO: usize = 25;
/// `browser.tabs.closeWindowWithLastTab`
pub const CLOSE_WINDOW_WITH_LAST_TAB: bool = true;
/// `browser.tabs.insertRelatedAfterCurrent`
pub const INSERT_RELATED_AFTER_CURRENT: bool = true;
/// `browser.zoom.siteSpecific`
pub const ZOOM_SITE_SPECIFIC: bool = true;
/// `browser.sessionstore.interval`, мс
pub const SESSIONSTORE_INTERVAL_MS: u64 = 15_000;
