use klin_shared::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fixup {
    Url(Url),
    Search(String),
}

const KNOWN_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "about",
    "data",
    "view-source",
    "blob",
    "moz-extension",
];

pub fn fixup(input: &str) -> Option<Fixup> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if let Some(rest) = input.strip_prefix('?') {
        let rest = rest.trim();
        return (!rest.is_empty()).then(|| Fixup::Search(rest.to_owned()));
    }
    if let Some((scheme, _)) = input.split_once(':') {
        let lower = scheme.to_ascii_lowercase();
        if KNOWN_SCHEMES.contains(&lower.as_str())
            && let Ok(url) = Url::parse(input)
        {
            return Some(Fixup::Url(url));
        }
    }
    if input.contains(char::is_whitespace) {
        return Some(Fixup::Search(input.to_owned()));
    }
    if let Some(url) = fixup_host_like(input) {
        return Some(Fixup::Url(url));
    }
    Some(Fixup::Search(input.to_owned()))
}

fn fixup_host_like(input: &str) -> Option<Url> {
    let end = input.find(['/', '?', '#']).unwrap_or(input.len());
    let authority = &input[..end];
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        Some(_) => return None,
        None => authority,
    };
    let host_lower = host.to_ascii_lowercase();
    let local = host_lower == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok();

    let domain = !local && looks_like_domain(&host_lower);
    if !local && !domain {
        return None;
    }

    let scheme = if local { "http" } else { "https" };
    Url::parse(&format!("{scheme}://{input}")).ok()
}

fn looks_like_domain(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty() || l.len() > 63) {
        return false;
    }
    let label_ok = |l: &&str| {
        !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    if !labels.iter().all(label_ok) {
        return false;
    }
    let tld = labels[labels.len() - 1];
    tld.starts_with("xn--") || (tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchEngine {
    pub name: String,
    pub template: String,
}

impl SearchEngine {
    pub fn search_url(&self, terms: &str) -> Option<Url> {
        let encoded: String = url::form_urlencoded::byte_serialize(terms.as_bytes()).collect();
        Url::parse(&self.template.replace("{searchTerms}", &encoded)).ok()
    }
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self {
            name: "Google".into(),
            template: "https://www.google.com/search?q={searchTerms}".into(),
        }
    }
}

pub fn to_url(input: &str, engine: &SearchEngine) -> Option<Url> {
    match fixup(input)? {
        Fixup::Url(url) => Some(url),
        Fixup::Search(terms) => engine.search_url(&terms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Option<Fixup> {
        Some(Fixup::Url(Url::parse(s).unwrap()))
    }

    fn search(s: &str) -> Option<Fixup> {
        Some(Fixup::Search(s.into()))
    }

    #[test]
    fn addresses() {
        assert_eq!(fixup("example.com"), url("https://example.com/"));
        assert_eq!(
            fixup("  mozilla.org/en-US/ "),
            url("https://mozilla.org/en-US/")
        );
        assert_eq!(fixup("http://example.com"), url("http://example.com/"));
        assert_eq!(fixup("localhost:8080/x"), url("http://localhost:8080/x"));
        assert_eq!(fixup("192.168.0.1"), url("http://192.168.0.1/"));
        assert_eq!(fixup("about:config"), url("about:config"));
        assert_eq!(fixup("яндекс.рф"), url("https://яндекс.рф/"));
    }

    #[test]
    fn searches() {
        assert_eq!(fixup("servo browser"), search("servo browser"));
        assert_eq!(fixup("servo"), search("servo"));
        assert_eq!(fixup("1.5"), search("1.5"));
        assert_eq!(fixup("? example.com"), search("example.com"));
        assert_eq!(fixup("   "), None);
    }

    #[test]
    fn search_url_is_encoded() {
        let url = to_url("rust & servo", &SearchEngine::default()).unwrap();
        assert_eq!(
            url.as_str(),
            "https://www.google.com/search?q=rust+%26+servo"
        );
    }
}
