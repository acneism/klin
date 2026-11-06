//! Адрес для показа в адресной строке — как в Firefox при `browser.urlbar.trimURLs`
//! и `browser.urlbar.trimHttps`: схема http(s) и «/» у корня скрываются, а у страниц
//! новой вкладки строка пустая.

use klin_shared::Url;

/// Страницы, на которых адресная строка пустая (в Firefox — `isInitialPage`).
const INITIAL_PAGES: &[&str] = &["about:newtab", "about:home", "about:blank"];

pub fn display_url(url: &Url) -> String {
    let s = url.as_str();
    if INITIAL_PAGES.contains(&s) {
        return String::new();
    }
    let trimmed = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"));
    match trimmed {
        Some(rest) => {
            let only_root = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
            let rest = if only_root {
                rest.strip_suffix('/').unwrap_or(rest)
            } else {
                rest
            };
            // Не прячем схему, если без неё адрес прочтётся как поиск или другой адрес.
            if rest.contains(' ') {
                s.to_owned()
            } else {
                rest.to_owned()
            }
        }
        None => s.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(s: &str) -> String {
        display_url(&Url::parse(s).unwrap())
    }

    #[test]
    fn trims_like_firefox() {
        assert_eq!(show("https://example.com/"), "example.com");
        assert_eq!(show("http://example.com/a/b?q=1"), "example.com/a/b?q=1");
        assert_eq!(show("https://example.com/?q"), "example.com/?q");
        assert_eq!(show("about:newtab"), "");
        assert_eq!(show("about:config"), "about:config");
        assert_eq!(show("file:///C:/x.html"), "file:///C:/x.html");
    }
}
