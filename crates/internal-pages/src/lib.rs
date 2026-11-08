pub mod reader;
pub mod resources;

pub const SCHEME: &str = "klin";

pub const READER: &str = "reader";

pub fn page(name: &str) -> Option<(&'static str, &'static [u8])> {
    match name {
        "newtab" | "home" | "blank" => Some(("text/html", include_bytes!("../pages/newtab.html"))),
        "preferences" => Some(("text/html", include_bytes!("../pages/preferences.html"))),
        "config" => Some(("text/html", include_bytes!("../pages/config.html"))),
        "library" => Some(("text/html", include_bytes!("../pages/library.html"))),
        _ => None,
    }
}

pub fn about_to_internal(url: &str) -> Option<String> {
    let rest = url.strip_prefix("about:")?;
    let name = rest.split(['?', '#']).next().unwrap_or(rest);
    (name == READER || page(name).is_some()).then(|| format!("{SCHEME}:{rest}"))
}

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
        assert_eq!(
            about_to_internal("about:reader?url=https%3A%2F%2Fe.com%2F").as_deref(),
            Some("klin:reader?url=https%3A%2F%2Fe.com%2F")
        );
        assert_eq!(
            about_to_internal("about:config?filter=x").as_deref(),
            Some("klin:config?filter=x")
        );
        assert_eq!(about_to_internal("https://example.com"), None);
    }
}
