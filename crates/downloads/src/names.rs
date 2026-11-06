use std::path::{Path, PathBuf};

use url::Url;

pub fn file_name_from_content_disposition(header: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for part in header.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "filename*" => {
                let mut pieces = value.splitn(3, '\'');
                let charset = pieces.next().unwrap_or("").to_ascii_lowercase();
                let _lang = pieces.next();
                if let Some(encoded) = pieces.next() {
                    let bytes: Vec<u8> = url::form_urlencoded::parse(
                        format!("x={}", encoded.replace('+', "%2B")).as_bytes(),
                    )
                    .next()
                    .map(|(_, v)| v.into_owned().into_bytes())
                    .unwrap_or_default();
                    if charset == "utf-8" || charset.is_empty() {
                        extended = String::from_utf8(bytes).ok();
                    }
                }
            }
            "filename" => {
                let v = value.trim_matches('"').replace("\\\"", "\"");
                plain = Some(v);
            }
            _ => {}
        }
    }
    extended.or(plain).filter(|n| !n.trim().is_empty())
}

pub fn file_name_from_url(url: &Url) -> String {
    let last = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let decoded = url::form_urlencoded::parse(format!("x={}", last.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    if decoded.trim().is_empty() {
        url.host_str().unwrap_or("download").to_owned()
    } else {
        decoded
    }
}

pub fn sanitize(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name = if trimmed.is_empty() {
        "download".to_owned()
    } else {
        trimmed.to_owned()
    };
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }

    if name.chars().count() > 240 {
        name = name.chars().take(240).collect();
    }
    name
}

pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let (stem, ext) = match name.find(".tar.") {
        Some(i) => (&name[..i], &name[i..]),
        None => match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        },
    };
    (1..)
        .map(|n| dir.join(format!("{stem}({n}){ext}")))
        .find(|p| !p.exists())
        .expect("infinite iterator")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition() {
        let cd = file_name_from_content_disposition;
        assert_eq!(
            cd("attachment; filename=\"report.pdf\"").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            cd("attachment; filename=plain.zip").as_deref(),
            Some("plain.zip")
        );
        assert_eq!(
            cd("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%D1%84%D0%B0%D0%B9%D0%BB.txt").as_deref(),
            Some("файл.txt")
        );
        assert_eq!(cd("inline"), None);
    }

    #[test]
    fn names_from_urls() {
        let u = |s| Url::parse(s).unwrap();
        assert_eq!(
            file_name_from_url(&u("https://e.com/files/setup%20v2.exe?x=1")),
            "setup v2.exe"
        );
        assert_eq!(file_name_from_url(&u("https://e.com/")), "e.com");
    }

    #[test]
    fn sanitizing() {
        assert_eq!(sanitize("a/b:c?.zip"), "a_b_c_.zip");
        assert_eq!(sanitize("  ..hidden. "), "hidden");
        assert_eq!(sanitize("con.txt"), "_con.txt");
        assert_eq!(sanitize(""), "download");
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("klin-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.zip"), dir.join("a.zip"));
        std::fs::write(dir.join("a.zip"), "").unwrap();
        assert_eq!(unique_path(&dir, "a.zip"), dir.join("a(1).zip"));
        std::fs::write(dir.join("b.tar.gz"), "").unwrap();
        assert_eq!(unique_path(&dir, "b.tar.gz"), dir.join("b(1).tar.gz"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
