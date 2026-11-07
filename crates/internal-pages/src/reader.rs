use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Article {
    pub url: String,
    pub title: String,
    pub byline: Option<String>,
    pub content: String,
    pub lang: Option<String>,
    pub dir: Option<String>,
    pub length: usize,
}

const STYLE: &str = include_str!("../pages/reader.css");

const READING_SPEED: &[(&str, u32, u32)] = &[
    ("en", 987, 118),
    ("ar", 612, 88),
    ("de", 920, 86),
    ("es", 1025, 127),
    ("fi", 1078, 121),
    ("fr", 998, 126),
    ("he", 833, 130),
    ("it", 950, 140),
    ("ja", 357, 56),
    ("nl", 978, 143),
    ("pl", 916, 126),
    ("pt", 913, 145),
    ("ru", 986, 175),
    ("sl", 885, 145),
    ("sv", 917, 156),
    ("tr", 1054, 156),
    ("zh", 255, 29),
];

pub fn reading_time(length: usize, lang: Option<&str>) -> (u32, u32) {
    let primary = lang
        .and_then(|l| l.split(['-', '_']).next())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let (_, cpm, variance) = READING_SPEED
        .iter()
        .find(|(code, _, _)| *code == primary)
        .unwrap_or(&READING_SPEED[0]);
    let length = length as f64;
    let slow = (length / f64::from(cpm - variance)).ceil() as u32;
    let fast = (length / f64::from(cpm + variance)).ceil() as u32;
    (fast, slow)
}

fn plural(n: u32, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    }
}

pub fn format_reading_time(fast: u32, slow: u32) -> String {
    let (fast, slow, words) = if slow < 120 {
        (fast, slow, ("минута", "минуты", "минут"))
    } else {
        (
            (f64::from(fast) / 60.0).round() as u32,
            (f64::from(slow) / 60.0).round() as u32,
            ("час", "часа", "часов"),
        )
    };
    let unit = plural(slow, words.0, words.1, words.2);
    if fast == slow {
        format!("~{slow} {unit}")
    } else {
        format!("{fast}–{slow} {unit}")
    }
}

pub fn strip_host(host: &str) -> &str {
    ["www.", "m.", "mobile."]
        .iter()
        .find_map(|prefix| host.strip_prefix(prefix))
        .unwrap_or(host)
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn sanitize(html: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["picture", "source", "video", "audio", "main", "section"])
        .add_generic_attributes(["class", "dir"])
        .add_tag_attributes("img", ["srcset", "sizes"])
        .add_tag_attributes("source", ["src", "srcset", "sizes", "type", "media"])
        .add_tag_attributes("video", ["src", "poster", "controls", "width", "height"])
        .add_tag_attributes("audio", ["src", "controls"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "data"]));
    builder.clean(html).to_string()
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.starts_with('[') {
        return host;
    }
    host.split(':').next().unwrap_or(host)
}

fn is_rtl(dir: Option<&str>) -> bool {
    dir == Some("rtl")
}

pub const CSP: &str = "default-src 'none'; img-src data: http: https:; media-src http: https:; style-src 'unsafe-inline'; object-src 'none'";

pub fn render(article: Option<&Article>, original: &str) -> String {
    let original_attr = escape(original);
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">\n"
    ));
    html.push_str("<meta name=\"viewport\" content=\"width=device-width\">\n");
    match article {
        Some(article) => {
            html.push_str(&format!("<title>{}</title>\n", escape(&article.title)));
            html.push_str(&format!("<style>{STYLE}</style>\n</head>\n"));
            html.push_str("<body class=\"auto sans-serif loaded\" style=\"--font-size: 20px; --content-width: 30em\">\n");
            let lang = article
                .lang
                .as_deref()
                .map(|l| format!(" lang=\"{}\"", escape(l)))
                .unwrap_or_default();
            let dir = if is_rtl(article.dir.as_deref()) {
                "rtl"
            } else {
                "ltr"
            };
            html.push_str(&format!(
                "<div class=\"container\" dir=\"{dir}\"{lang} style=\"--line-height: 1.6em\">\n"
            ));
            html.push_str("<div class=\"header reader-header reader-show-element\">\n");
            html.push_str(&format!(
                "<a class=\"domain reader-domain\" href=\"{original_attr}\">{}</a>\n",
                escape(strip_host(host_of(&article.url)))
            ));
            html.push_str("<div class=\"domain-border\"></div>\n");
            html.push_str(&format!(
                "<h1 class=\"reader-title\">{}</h1>\n",
                escape(&article.title)
            ));
            html.push_str(&format!(
                "<div class=\"credits reader-credits\">{}</div>\n",
                escape(article.byline.as_deref().unwrap_or(""))
            ));
            let (fast, slow) = reading_time(article.length, article.lang.as_deref());
            html.push_str(&format!(
                "<div class=\"meta-data\"><div class=\"reader-estimated-time\" dir=\"ltr\">{}</div></div>\n",
                format_reading_time(fast, slow)
            ));
            html.push_str("</div>\n<hr>\n<div class=\"content\">\n");
            html.push_str("<div class=\"moz-reader-content reader-show-element\">");
            html.push_str(&sanitize(&article.content));
            html.push_str("</div>\n</div>\n</div>\n");
        }
        None => {
            html.push_str("<title>Не удалось загрузить статью со страницы</title>\n");
            html.push_str(&format!("<style>{STYLE}</style>\n</head>\n"));
            html.push_str("<body class=\"auto sans-serif\" style=\"--font-size: 20px; --content-width: 30em\">\n");
            html.push_str("<div class=\"container\">\n<div class=\"reader-message reader-show-element\" style=\"display: block\">");
            html.push_str("Не удалось загрузить статью со страницы");
            if !original.is_empty() {
                html.push_str(&format!(
                    "<p><a href=\"{original_attr}\">Открыть исходную страницу</a></p>"
                ));
            }
            html.push_str("</div>\n</div>\n");
        }
    }
    html.push_str("</body>\n</html>\n");
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_time_matches_firefox() {
        assert_eq!(reading_time(9870, Some("en")), (9, 12));
        assert_eq!(reading_time(9870, Some("ru-RU")), (9, 13));
        assert_eq!(reading_time(9870, None), (9, 12));
        assert_eq!(reading_time(0, None), (0, 0));
    }

    #[test]
    fn formats_range_in_russian() {
        assert_eq!(format_reading_time(9, 12), "9–12 минут");
        assert_eq!(format_reading_time(1, 2), "1–2 минуты");
        assert_eq!(format_reading_time(1, 1), "~1 минута");
        assert_eq!(format_reading_time(20, 21), "20–21 минута");
        assert_eq!(format_reading_time(100, 130), "~2 часа");
        assert_eq!(format_reading_time(100, 400), "2–7 часов");
    }

    #[test]
    fn strips_common_subdomains() {
        assert_eq!(strip_host("www.example.com"), "example.com");
        assert_eq!(strip_host("m.example.com"), "example.com");
        assert_eq!(strip_host("mobile.example.com"), "example.com");
        assert_eq!(strip_host("news.example.com"), "news.example.com");
        assert_eq!(
            host_of("https://user@www.example.com:8080/a?b"),
            "www.example.com"
        );
    }

    #[test]
    fn sanitizer_drops_scripts_and_handlers() {
        let dirty = r#"<p onclick="x()">Hi<script>alert(1)</script><a href="javascript:alert(1)">l</a><img src="https://e.com/a.png" onerror="x()"><form><input></form><iframe src="https://e.com"></iframe></p>"#;
        let clean = sanitize(dirty);
        assert!(!clean.contains("script"));
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("javascript:"));
        assert!(!clean.contains("<form"));
        assert!(!clean.contains("<iframe"));
        assert!(clean.contains("https://e.com/a.png"));
    }

    #[test]
    fn renders_article_with_escaped_metadata() {
        let article = Article {
            url: "https://www.example.com/post".into(),
            title: "<b>Title</b>".into(),
            byline: Some("Author".into()),
            content: "<p>Body</p>".into(),
            lang: Some("en".into()),
            dir: None,
            length: 2000,
        };
        let html = render(Some(&article), "https://www.example.com/post");
        assert!(html.contains("<h1 class=\"reader-title\">&lt;b&gt;Title&lt;/b&gt;</h1>"));
        assert!(html.contains(">example.com</a>"));
        assert!(html.contains("<p>Body</p>"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("2–3 минуты"));
    }

    #[test]
    fn renders_error_page_without_article() {
        let html = render(None, "https://example.com/\"x");
        assert!(html.contains("Не удалось загрузить статью со страницы"));
        assert!(html.contains("href=\"https://example.com/&quot;x\""));
    }
}
