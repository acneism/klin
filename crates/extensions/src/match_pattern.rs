use url::Url;

const ALL_SCHEMES: &[&str] = &["http", "https", "ws", "wss", "file", "data"];
const WILDCARD_SCHEMES: &[&str] = &["http", "https", "ws", "wss"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPattern {
    schemes: Vec<String>,
    host: Option<HostPattern>,
    path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HostPattern {
    Any,
    Exact(String),
    Subdomains(String),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("недопустимый шаблон адреса: {0}")]
pub struct PatternError(pub String);

fn glob_with(pattern: &str, text: &str, question: bool) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && ((question && p[pi] == '?') || p[pi] == t[ti]) && p[pi] != '*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn glob(pattern: &str, text: &str) -> bool {
    glob_with(pattern, text, true)
}

impl MatchPattern {
    pub fn parse(pattern: &str) -> Result<Self, PatternError> {
        let error = || PatternError(pattern.to_owned());
        if pattern == "<all_urls>" {
            return Ok(Self {
                schemes: ALL_SCHEMES.iter().map(|s| (*s).to_owned()).collect(),
                host: Some(HostPattern::Any),
                path: "*".into(),
            });
        }
        let (scheme, rest) = pattern.split_once("://").ok_or_else(error)?;
        let schemes: Vec<String> = match scheme {
            "*" => WILDCARD_SCHEMES.iter().map(|s| (*s).to_owned()).collect(),
            s if ALL_SCHEMES.contains(&s) => vec![s.to_owned()],
            _ => return Err(error()),
        };
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => return Err(error()),
        };
        let host = if scheme == "file" || scheme == "data" {
            None
        } else if host == "*" {
            Some(HostPattern::Any)
        } else if let Some(domain) = host.strip_prefix("*.") {
            if domain.is_empty() || domain.contains('*') {
                return Err(error());
            }
            Some(HostPattern::Subdomains(domain.to_ascii_lowercase()))
        } else if host.is_empty()
            || host.contains('*')
            || host.contains(':') && !host.starts_with('[')
        {
            return Err(error());
        } else {
            Some(HostPattern::Exact(host.to_ascii_lowercase()))
        };
        Ok(Self {
            schemes,
            host,
            path: path.to_owned(),
        })
    }

    pub fn matches(&self, url: &Url) -> bool {
        if !self.schemes.iter().any(|s| s == url.scheme()) {
            return false;
        }
        let host_ok = match &self.host {
            None => true,
            Some(HostPattern::Any) => true,
            Some(HostPattern::Exact(h)) => {
                url.host_str().is_some_and(|u| u.eq_ignore_ascii_case(h))
            }
            Some(HostPattern::Subdomains(d)) => url.host_str().is_some_and(|u| {
                let u = u.to_ascii_lowercase();
                u == *d || u.ends_with(&format!(".{d}"))
            }),
        };
        if !host_ok {
            return false;
        }
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        glob_with(&self.path, &path, false)
    }

    pub fn to_regex(&self) -> String {
        let schemes = self.schemes.join("|");
        let host = match &self.host {
            None => String::new(),
            Some(HostPattern::Any) => r"(?:[^/@]*@)?[^/:]*(?::\d+)?".into(),
            Some(HostPattern::Exact(h)) => format!(r"(?:[^/@]*@)?{}(?::\d+)?", escape(h)),
            Some(HostPattern::Subdomains(d)) => {
                format!(r"(?:[^/@]*@)?(?:[^/:]*\.)?{}(?::\d+)?", escape(d))
            }
        };
        let path: String = self
            .path
            .split('*')
            .map(escape)
            .collect::<Vec<_>>()
            .join(".*");
        format!("^(?:{schemes}):\\/\\/{host}{path}$")
    }

    pub fn matches_host(&self, url: &Url) -> bool {
        let mut any_path = self.clone();
        any_path.path = "*".into();
        any_path.matches(url)
    }
}

pub fn glob_matches(pattern: &str, text: &str) -> bool {
    glob(pattern, text)
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\^$.|?*+()[]{}/-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn glob_to_regex(glob: &str) -> String {
    let mut out = String::from("^");
    for c in glob.chars() {
        match c {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c => out.push_str(&escape(&c.to_string())),
        }
    }
    out.push('$');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, url: &str) -> bool {
        MatchPattern::parse(pattern)
            .unwrap()
            .matches(&Url::parse(url).unwrap())
    }

    #[test]
    fn matches_like_firefox() {
        assert!(m("<all_urls>", "https://example.com/"));
        assert!(m("<all_urls>", "file:///C:/a.txt"));
        assert!(!m("<all_urls>", "about:config"));
        assert!(m("*://*.mozilla.org/*", "https://mozilla.org/"));
        assert!(m("*://*.mozilla.org/*", "http://a.b.mozilla.org/x?y"));
        assert!(!m("*://*.mozilla.org/*", "https://notmozilla.org/"));
        assert!(!m("*://*.mozilla.org/*", "ftp://mozilla.org/"));
        assert!(!m("*://*/*", "file:///a"));
        assert!(m(
            "https://example.org/foo/*",
            "https://example.org/foo/bar.html"
        ));
        assert!(!m("https://example.org/foo/*", "https://example.org/bar"));
        assert!(m("https://example.org/*.js", "https://example.org/a/b.js"));
        assert!(!m(
            "https://example.org/*.js",
            "https://example.org/a/b.js?x"
        ));
        assert!(!m("https://example.org/a?c", "https://example.org/abc"));
        assert!(m("*://example.org/a?b", "https://example.org/a?b"));
        assert!(m("file:///blah/*", "file:///blah/x"));
    }

    #[test]
    fn rejects_invalid_patterns() {
        for bad in [
            "https://example.org",
            "ftp://example.org/*",
            "https://*foo/*",
            "https://foo.*.bar/*",
            "*://*.",
            "https://example.org:8080/*",
        ] {
            assert!(MatchPattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn globs() {
        assert!(glob("*", ""));
        assert!(glob("a*c", "abbbc"));
        assert!(glob("a?c", "abc"));
        assert!(!glob("a?c", "ac"));
        assert!(glob("*.example.*", "www.example.com"));
    }
}
