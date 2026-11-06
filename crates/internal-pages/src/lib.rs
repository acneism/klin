//! Внутренние страницы (about:newtab, about:home, страницы ошибок).
//!
//! Servo не разрешает регистрировать схему `about:`, поэтому страницы отдаются через
//! собственную схему [`SCHEME`], а адресная строка показывает привычные about:-адреса.

/// Схема, через которую движок получает внутренние страницы.
pub const SCHEME: &str = "klin";

/// Страница по имени (`newtab` для about:newtab): MIME-тип и содержимое.
pub fn page(name: &str) -> Option<(&'static str, &'static [u8])> {
    match name {
        "newtab" | "home" | "blank" => Some(("text/html", include_bytes!("../pages/newtab.html"))),
        _ => None,
    }
}

/// about:name → klin:name. Остальные адреса не меняются.
pub fn about_to_internal(url: &str) -> Option<String> {
    let name = url.strip_prefix("about:")?;
    let name = name.split(['?', '#']).next().unwrap_or(name);
    page(name).map(|_| format!("{SCHEME}:{name}"))
}

/// klin:name → about:name, для показа в адресной строке.
pub fn internal_to_about(url: &str) -> Option<String> {
    url.strip_prefix(SCHEME)?
        .strip_prefix(':')
        .map(|name| format!("about:{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_mapping_round_trips() {
        assert_eq!(
            about_to_internal("about:newtab").as_deref(),
            Some("klin:newtab")
        );
        assert_eq!(
            internal_to_about("klin:newtab").as_deref(),
            Some("about:newtab")
        );
        assert_eq!(about_to_internal("about:unknown"), None);
        assert_eq!(about_to_internal("https://example.com"), None);
    }
}
