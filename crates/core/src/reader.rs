use klin_shared::Url;

const BLOCKED_HOSTS: &[&str] = &[
    "amazon.com",
    "github.com",
    "mail.google.com",
    "pinterest.com",
    "reddit.com",
    "twitter.com",
    "youtube.com",
    "app.slack.com",
];

pub fn should_check_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https" | "file") {
        return false;
    }
    if url.scheme().starts_with("http") {
        let host = url.host_str().unwrap_or("");
        if BLOCKED_HOSTS.iter().any(|blocked| host.ends_with(blocked)) {
            let path = url.path();
            return host == "github.com"
                && !path.contains("/projects")
                && !path.contains("/issues");
        }
        if url.path() == "/" {
            return false;
        }
    }
    true
}

fn encode_uri_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn reader_url(url: &Url) -> Url {
    let spec = format!("about:reader?url={}", encode_uri_component(url.as_str()));
    Url::parse(&spec).unwrap_or_else(|_| url.clone())
}

pub fn original_url(url: &Url) -> Option<Url> {
    if url.scheme() != "about" || url.path() != "reader" {
        return None;
    }
    let (_, value) = url.query_pairs().find(|(key, _)| key == "url")?;
    Url::parse(&value).ok()
}

pub fn is_reader(url: &Url) -> bool {
    url.scheme() == "about" && url.path() == "reader"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn checks_uris_like_firefox() {
        assert!(should_check_url(&u("https://example.com/article")));
        assert!(!should_check_url(&u("https://example.com/")));
        assert!(!should_check_url(&u("about:newtab")));
        assert!(should_check_url(&u("file:///C:/a.html")));
        assert!(!should_check_url(&u("https://www.youtube.com/watch?v=1")));
        assert!(!should_check_url(&u("https://www.reddit.com/r/rust/")));
        assert!(should_check_url(&u("https://github.com/servo/servo")));
        assert!(!should_check_url(&u(
            "https://github.com/servo/servo/issues/1"
        )));
        assert!(!should_check_url(&u("https://gist.github.com/a/b")));
    }

    #[test]
    fn reader_url_round_trips() {
        let page = u("https://example.com/a b?x=1&y=2#frag");
        let reader = reader_url(&page);
        assert_eq!(
            reader.as_str(),
            "about:reader?url=https%3A%2F%2Fexample.com%2Fa%2520b%3Fx%3D1%26y%3D2%23frag"
        );
        assert!(is_reader(&reader));
        assert_eq!(original_url(&reader), Some(page));
        assert_eq!(original_url(&u("about:config")), None);
        assert_eq!(original_url(&u("https://example.com/?url=x")), None);
    }
}
