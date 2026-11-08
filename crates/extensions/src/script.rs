use serde_json::{Value, json};

use crate::manifest::{Manifest, Messages, RunAt};
use crate::match_pattern::glob_to_regex;
use crate::package::Package;

const RUNTIME: &str = include_str!("../js/runtime.js");

pub struct ScriptConfig<'a> {
    pub uuid: &'a str,
    pub id: &'a str,
    pub manifest: &'a Manifest,
    pub package: &'a Package,
    pub ui_language: &'a str,
    pub browser_version: &'a str,
}

fn run_at(run_at: RunAt) -> &'static str {
    match run_at {
        RunAt::DocumentStart => "document_start",
        RunAt::DocumentEnd => "document_end",
        RunAt::DocumentIdle => "document_idle",
    }
}

pub fn user_script(config: &ScriptConfig<'_>) -> String {
    let manifest = config.manifest;
    let messages = Messages::load(
        config.package,
        manifest.default_locale.as_deref(),
        config.ui_language,
    );
    let mut scripts = Vec::new();
    let mut entries = Vec::new();
    for content in &manifest.content_scripts {
        let css: Vec<String> = content
            .css
            .iter()
            .filter_map(|path| config.package.read_text(path).ok())
            .collect();
        scripts.push(json!({
            "matches": content.matches.iter().map(|m| m.to_regex()).collect::<Vec<_>>(),
            "excludeMatches": content.exclude_matches.iter().map(|m| m.to_regex()).collect::<Vec<_>>(),
            "includeGlobs": content.include_globs.iter().map(|g| glob_to_regex(g)).collect::<Vec<_>>(),
            "excludeGlobs": content.exclude_globs.iter().map(|g| glob_to_regex(g)).collect::<Vec<_>>(),
            "runAt": run_at(content.run_at),
            "css": css,
        }));
        let mut body = String::new();
        for path in &content.js {
            match config.package.read_text(path) {
                Ok(code) => {
                    body.push_str(&code);
                    body.push_str("\n;\n");
                }
                Err(error) => log::warn!("{}: {error}", config.id),
            }
        }
        entries.push(format!("function (browser, chrome) {{\n{body}\n}}"));
    }
    let settings = json!({
        "uuid": config.uuid,
        "id": config.id,
        "manifest": manifest.raw,
        "manifestVersion": manifest.manifest_version,
        "messages": messages.to_json(),
        "placeholders": messages.placeholders_json(),
        "uiLanguage": config.ui_language,
        "browserVersion": config.browser_version,
        "scripts": scripts,
    });
    let settings = serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into());
    format!(
        "({})({}, [{}]);",
        RUNTIME.trim().trim_end_matches(';'),
        settings.replace("</", "<\\/"),
        entries.join(",\n")
    )
}

pub fn background_page(manifest: &Manifest) -> Option<String> {
    match manifest.background.as_ref()? {
        crate::manifest::Background::Scripts(scripts) => {
            let tags: String = scripts
                .iter()
                .map(|s| {
                    format!(
                        "<script src=\"/{}\"></script>\n",
                        s.trim_start_matches('/').replace('"', "&quot;")
                    )
                })
                .collect();
            Some(format!(
                "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body>\n{tags}</body>\n</html>\n"
            ))
        }
        crate::manifest::Background::Page(_) => None,
    }
}

pub fn value_for_js(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(files: &[(&str, &str)]) -> (tempfile::TempDir, Package, Manifest) {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = Package::open(dir.path()).unwrap();
        let (manifest, _) = Manifest::load(&package, "ru").unwrap();
        (dir, package, manifest)
    }

    fn run_in_node(script: &str, url: &str, extra: &str) -> Option<String> {
        let harness = format!(
            r#"
const sent = [];
const parsed = new URL({url:?});
globalThis.location = {{ href: parsed.href, protocol: parsed.protocol, host: parsed.host }};
globalThis.window = globalThis;
globalThis.top = globalThis;
globalThis.console = {{ debug: (m) => sent.push(JSON.parse(m.slice(10))), error: (e) => sent.push({{ error: String(e) }}) }};
globalThis.document = {{ readyState: "loading", addEventListener: (t, f) => {{ globalThis.ready = f; }}, head: {{ appendChild: (n) => sent.push({{ css: n.textContent }}) }}, createElement: () => ({{}}) }};
globalThis.setTimeout = (f) => f();
{script}
if (globalThis.ready) globalThis.ready();
{extra}
Promise.resolve().then(() => 0).then(() => 0).then(() => process.stdout.write(JSON.stringify(sent)));
"#
        );
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(harness)
            .output()
            .ok()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Some(String::from_utf8(output.stdout).unwrap())
    }

    #[test]
    fn content_scripts_run_on_matching_pages_only() {
        let (_dir, package, manifest) = extension(&[
            (
                "manifest.json",
                r#"{"manifest_version": 2, "name": "T", "version": "1",
                    "content_scripts": [{"matches": ["*://*.example.com/*"], "exclude_matches": ["*://*/private/*"], "js": ["a.js"], "css": ["a.css"]}]}"#,
            ),
            (
                "a.js",
                "var seen = 1; browser.storage.local.get('k'); chrome.runtime.sendMessage({hi: location.href});",
            ),
            ("a.css", "body { color: red }"),
        ]);
        let script = user_script(&ScriptConfig {
            uuid: "1234-uuid",
            id: "t@example.com",
            manifest: &manifest,
            package: &package,
            ui_language: "ru",
            browser_version: "0.3",
        });
        let Some(out) = run_in_node(&script, "https://www.example.com/page", "") else {
            eprintln!("node is not installed; skipping");
            return;
        };
        let sent: Value = serde_json::from_str(&out).unwrap();
        let sent = sent.as_array().unwrap();
        assert_eq!(sent[0]["type"], "hello");
        assert_eq!(sent[0]["context"], "content");
        assert_eq!(sent[0]["ext"], "1234-uuid");
        let token = sent[0]["token"].as_str().unwrap();
        assert_eq!(token.len(), 32);
        assert_eq!(sent[1]["css"], "body { color: red }");
        assert_eq!(sent[2]["api"], "storage.get");
        assert_eq!(sent[2]["args"], json!(["local", "k"]));
        assert_eq!(sent[3]["api"], "runtime.sendMessage");
        assert_eq!(
            sent[3]["args"],
            json!([{"hi": "https://www.example.com/page"}])
        );
        assert!(sent.iter().all(|m| m.get("error").is_none()), "{sent:?}");

        let skipped = run_in_node(&script, "https://www.example.com/private/x", "").unwrap();
        assert_eq!(skipped, "[]");
        let other = run_in_node(&script, "https://example.org/", "").unwrap();
        assert_eq!(other, "[]");
    }

    #[test]
    fn extension_pages_get_the_full_api_and_replies() {
        let (_dir, package, manifest) = extension(&[
            (
                "manifest.json",
                r#"{"manifest_version": 3, "name": "T", "version": "1", "default_locale": "en"}"#,
            ),
            (
                "_locales/en/messages.json",
                r#"{"greet": {"message": "Hello $who$, $1!", "placeholders": {"who": {"content": "dear"}}}}"#,
            ),
        ]);
        let script = user_script(&ScriptConfig {
            uuid: "uuid-1",
            id: "t@example.com",
            manifest: &manifest,
            package: &package,
            ui_language: "ru",
            browser_version: "0.3",
        });
        let extra = r#"
const hello = sent[0];
browser.tabs.query({active: true}).then((tabs) => sent.push({ tabs }));
chrome.storage.local.get(null, (items) => sent.push({ items, lastError: chrome.runtime.lastError }));
window[hello.entry](hello.token, { type: "reply", id: 1, result: [{ id: 7 }] });
window[hello.entry]("wrong-token", { type: "reply", id: 2, result: "forged" });
window[hello.entry](hello.token, { type: "reply", id: 2, result: { a: 1 } });
sent.push({ msg: browser.i18n.getMessage("GREET", ["Ann"]), resource: browser.runtime.getURL("/p.html"), action: typeof browser.action.setBadgeText });
"#;
        let Some(out) = run_in_node(&script, "moz-extension://uuid-1/popup.html", extra) else {
            return;
        };
        let sent: Value = serde_json::from_str(&out).unwrap();
        let sent = sent.as_array().unwrap();
        assert_eq!(sent[0]["context"], "page");
        assert_eq!(sent[1]["api"], "tabs.query");
        assert_eq!(sent[2]["api"], "storage.get");
        let find = |key: &str| sent.iter().find(|m| m.get(key).is_some()).unwrap().clone();
        assert_eq!(find("msg")["msg"], "Hello dear, Ann!");
        assert_eq!(
            find("resource")["resource"],
            "moz-extension://uuid-1/p.html"
        );
        assert_eq!(find("action")["action"], "function");
        assert_eq!(find("tabs")["tabs"], json!([{"id": 7}]));
        assert_eq!(find("items")["items"], json!({"a": 1}));
        assert!(find("items").get("lastError").is_none());
    }
}
