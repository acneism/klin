use std::collections::HashMap;

use serde_json::Value;

use crate::match_pattern::MatchPattern;
use crate::package::{Package, PackageError};

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("manifest.json не является корректным JSON: {0}")]
    Json(String),
    #[error("в manifest.json нет поля {0}")]
    Missing(&'static str),
    #[error("manifest_version {0} не поддерживается")]
    Version(i64),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunAt {
    DocumentStart,
    DocumentEnd,
    #[default]
    DocumentIdle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentScript {
    pub matches: Vec<MatchPattern>,
    pub exclude_matches: Vec<MatchPattern>,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub js: Vec<String>,
    pub css: Vec<String>,
    pub run_at: RunAt,
    pub all_frames: bool,
    pub match_about_blank: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Background {
    Scripts(Vec<String>),
    Page(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Action {
    pub title: Option<String>,
    pub popup: Option<String>,
    pub icons: Vec<(u32, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub manifest_version: i64,
    pub id: Option<String>,
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_locale: Option<String>,
    pub permissions: Vec<String>,
    pub host_permissions: Vec<MatchPattern>,
    pub content_scripts: Vec<ContentScript>,
    pub background: Option<Background>,
    pub action: Option<Action>,
    pub icons: Vec<(u32, String)>,
    pub options_page: Option<String>,
    pub raw: Value,
}

pub struct Messages(
    HashMap<String, String>,
    HashMap<String, HashMap<String, String>>,
);

impl Messages {
    pub fn load(package: &Package, default_locale: Option<&str>, ui_locale: &str) -> Self {
        let mut map = HashMap::new();
        let mut placeholders: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut locales = vec![ui_locale.replace('-', "_")];
        if let Some((lang, _)) = ui_locale.split_once(['-', '_']) {
            locales.push(lang.to_owned());
        }
        if let Some(default) = default_locale {
            locales.push(default.to_owned());
        }
        for locale in locales.iter().rev() {
            let Ok(text) = package.read_text(&format!("_locales/{locale}/messages.json")) else {
                continue;
            };
            let Ok(Value::Object(messages)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (key, value) in messages {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    map.insert(key.to_ascii_lowercase(), message.to_owned());
                }
                if let Some(Value::Object(defined)) = value.get("placeholders") {
                    let entry = placeholders.entry(key.to_ascii_lowercase()).or_default();
                    for (name, placeholder) in defined {
                        if let Some(content) = placeholder.get("content").and_then(Value::as_str) {
                            entry.insert(name.to_ascii_lowercase(), content.to_owned());
                        }
                    }
                }
            }
        }
        Self(map, placeholders)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(&key.to_ascii_lowercase()).map(String::as_str)
    }

    pub fn localize(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("__MSG_") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 6..];
            match after.find("__") {
                Some(end) => {
                    let key = &after[..end];
                    out.push_str(self.get(key).unwrap_or(""));
                    rest = &after[end + 2..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        )
    }

    pub fn placeholders_json(&self) -> Value {
        serde_json::to_value(&self.1).unwrap_or(Value::Null)
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn patterns(value: Option<&Value>, warnings: &mut Vec<String>) -> Vec<MatchPattern> {
    strings(value)
        .iter()
        .filter_map(|p| match MatchPattern::parse(p) {
            Ok(pattern) => Some(pattern),
            Err(error) => {
                warnings.push(error.to_string());
                None
            }
        })
        .collect()
}

fn icons(value: Option<&Value>) -> Vec<(u32, String)> {
    let mut icons: Vec<(u32, String)> = match value {
        Some(Value::Object(map)) => map
            .iter()
            .filter_map(|(size, path)| Some((size.parse().ok()?, path.as_str()?.to_owned())))
            .collect(),
        Some(Value::String(path)) => vec![(16, path.clone())],
        _ => Vec::new(),
    };
    icons.sort();
    icons
}

fn strip_json_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for next in chars.by_ref() {
                    if prev == '*' && next == '/' {
                        break;
                    }
                    prev = next;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

impl Manifest {
    pub fn load(package: &Package, ui_locale: &str) -> Result<(Self, Vec<String>), ManifestError> {
        let text = package.read_text("manifest.json")?;
        let raw: Value = serde_json::from_str(&strip_json_comments(&text))
            .map_err(|e| ManifestError::Json(e.to_string()))?;
        let mut warnings = Vec::new();
        let version = raw
            .get("manifest_version")
            .and_then(Value::as_i64)
            .ok_or(ManifestError::Missing("manifest_version"))?;
        if version != 2 && version != 3 {
            return Err(ManifestError::Version(version));
        }
        let default_locale = raw
            .get("default_locale")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let messages = Messages::load(package, default_locale.as_deref(), ui_locale);
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(|s| messages.localize(s))
        };
        let name = text("name").ok_or(ManifestError::Missing("name"))?;
        let ext_version = raw
            .get("version")
            .and_then(Value::as_str)
            .ok_or(ManifestError::Missing("version"))?
            .to_owned();
        let gecko = raw
            .get("browser_specific_settings")
            .or_else(|| raw.get("applications"))
            .and_then(|b| b.get("gecko"));
        let id = gecko
            .and_then(|g| g.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut permissions = Vec::new();
        let mut host_permissions = Vec::new();
        let mut split = |value: Option<&Value>| {
            for permission in strings(value) {
                if permission == "<all_urls>" || permission.contains("://") {
                    match MatchPattern::parse(&permission) {
                        Ok(pattern) => host_permissions.push(pattern),
                        Err(error) => warnings.push(error.to_string()),
                    }
                } else {
                    permissions.push(permission);
                }
            }
        };
        split(raw.get("permissions"));
        split(raw.get("host_permissions"));
        let mut content_scripts = Vec::new();
        for entry in raw
            .get("content_scripts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let matches = patterns(entry.get("matches"), &mut warnings);
            if matches.is_empty() {
                warnings.push("content_scripts без matches пропущены".into());
                continue;
            }
            let run_at = match entry.get("run_at").and_then(Value::as_str) {
                Some("document_start") => RunAt::DocumentStart,
                Some("document_end") => RunAt::DocumentEnd,
                _ => RunAt::DocumentIdle,
            };
            content_scripts.push(ContentScript {
                matches,
                exclude_matches: patterns(entry.get("exclude_matches"), &mut warnings),
                include_globs: strings(entry.get("include_globs")),
                exclude_globs: strings(entry.get("exclude_globs")),
                js: strings(entry.get("js")),
                css: strings(entry.get("css")),
                run_at,
                all_frames: entry.get("all_frames").and_then(Value::as_bool) == Some(true),
                match_about_blank: entry.get("match_about_blank").and_then(Value::as_bool)
                    == Some(true),
            });
        }
        let background = raw.get("background").and_then(|b| {
            let scripts = strings(b.get("scripts"));
            if !scripts.is_empty() {
                Some(Background::Scripts(scripts))
            } else if let Some(page) = b.get("page").and_then(Value::as_str) {
                Some(Background::Page(page.to_owned()))
            } else {
                if b.get("service_worker").is_some() {
                    warnings.push(
                        "background.service_worker не поддерживается, как и в Firefox".into(),
                    );
                }
                None
            }
        });
        let action = raw
            .get(if version == 3 {
                "action"
            } else {
                "browser_action"
            })
            .or_else(|| raw.get("browser_action"))
            .map(|a| Action {
                title: a
                    .get("default_title")
                    .and_then(Value::as_str)
                    .map(|t| messages.localize(t)),
                popup: a
                    .get("default_popup")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned),
                icons: icons(a.get("default_icon")),
            });
        let options_page = raw
            .get("options_ui")
            .and_then(|o| o.get("page"))
            .or_else(|| raw.get("options_page"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok((
            Manifest {
                manifest_version: version,
                id,
                name,
                version: ext_version,
                description: text("description").unwrap_or_default(),
                default_locale,
                permissions,
                host_permissions,
                content_scripts,
                background,
                action,
                icons: icons(raw.get("icons")),
                options_page,
                raw,
            },
            warnings,
        ))
    }

    pub fn has_permission(&self, name: &str) -> bool {
        self.permissions.iter().any(|p| p == name)
    }

    pub fn best_icon(&self, size: u32) -> Option<&str> {
        let icons = self
            .action
            .as_ref()
            .map(|a| a.icons.as_slice())
            .filter(|i| !i.is_empty())
            .unwrap_or(&self.icons);
        icons
            .iter()
            .find(|(s, _)| *s >= size)
            .or(icons.last())
            .map(|(_, p)| p.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(files: &[(&str, &str)]) -> (tempfile::TempDir, Package) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = Package::open(dir.path()).unwrap();
        (dir, package)
    }

    #[test]
    fn parses_a_localized_mv2_manifest() {
        let (_dir, package) = package(&[
            (
                "manifest.json",
                r#"{
                    // comment
                    "manifest_version": 2,
                    "name": "__MSG_extName__",
                    "version": "1.2",
                    "default_locale": "en",
                    "browser_specific_settings": {"gecko": {"id": "test@example.com"}},
                    "permissions": ["storage", "tabs", "<all_urls>", "*://*.example.com/*"],
                    "background": {"scripts": ["bg.js"]},
                    "browser_action": {"default_popup": "popup.html", "default_title": "T", "default_icon": {"16": "i16.png", "32": "i32.png"}},
                    "content_scripts": [{"matches": ["https://*/*"], "js": ["a.js"], "run_at": "document_start", "all_frames": true}]
                }"#,
            ),
            (
                "_locales/en/messages.json",
                r#"{"extName": {"message": "English"}}"#,
            ),
            (
                "_locales/ru/messages.json",
                r#"{"extname": {"message": "Русское"}}"#,
            ),
        ]);
        let (manifest, warnings) = Manifest::load(&package, "ru-RU").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(manifest.name, "Русское");
        assert_eq!(manifest.id.as_deref(), Some("test@example.com"));
        assert_eq!(manifest.permissions, ["storage", "tabs"]);
        assert_eq!(manifest.host_permissions.len(), 2);
        assert_eq!(
            manifest.background,
            Some(Background::Scripts(vec!["bg.js".into()]))
        );
        let action = manifest.action.as_ref().unwrap();
        assert_eq!(action.popup.as_deref(), Some("popup.html"));
        assert_eq!(manifest.best_icon(20), Some("i32.png"));
        assert_eq!(manifest.content_scripts[0].run_at, RunAt::DocumentStart);
        assert!(manifest.content_scripts[0].all_frames);
        let (english, _) = Manifest::load(&package, "de").unwrap();
        assert_eq!(english.name, "English");
    }

    #[test]
    fn parses_mv3_and_rejects_bad_manifests() {
        let (_dir, mv3) = package(&[(
            "manifest.json",
            r#"{"manifest_version": 3, "name": "N", "version": "1", "action": {"default_popup": "p.html"}, "host_permissions": ["https://a.example/*"], "background": {"service_worker": "sw.js"}}"#,
        )]);
        let (manifest, warnings) = Manifest::load(&mv3, "ru").unwrap();
        assert_eq!(manifest.action.unwrap().popup.as_deref(), Some("p.html"));
        assert_eq!(manifest.host_permissions.len(), 1);
        assert_eq!(manifest.background, None);
        assert_eq!(warnings.len(), 1);
        let (_dir, bad) = package(&[(
            "manifest.json",
            r#"{"manifest_version": 4, "name": "x", "version": "1"}"#,
        )]);
        assert!(matches!(
            Manifest::load(&bad, "ru"),
            Err(ManifestError::Version(4))
        ));
        let (_dir, missing) = package(&[(
            "manifest.json",
            r#"{"manifest_version": 2, "version": "1"}"#,
        )]);
        assert!(matches!(
            Manifest::load(&missing, "ru"),
            Err(ManifestError::Missing("name"))
        ));
    }
}
