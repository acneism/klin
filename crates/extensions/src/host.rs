use std::collections::HashMap;
use std::path::PathBuf;

use klin_shared::{TabId, Url};
use serde_json::{Map, Value, json};

use crate::manifest::Manifest;
use crate::package::Package;
use crate::storage::StorageArea;

pub const MARK: &str = "\u{1}klin-ext\u{1}";

pub const BACKGROUND_PAGE: &str = "_generated_background_page.html";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Content,
    Page,
}

#[derive(Clone, Debug)]
struct Session {
    token: String,
    entry: String,
    context: Context,
    url: String,
}

pub struct Extension {
    pub id: String,
    pub uuid: String,
    pub manifest: Manifest,
    pub package: Package,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRef {
    pub tab: TabId,
    pub uuid: String,
    pub id: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Eval {
        tab: TabId,
        script: String,
    },
    Api {
        call: CallRef,
        extension_id: String,
        api: String,
        args: Vec<Value>,
    },
}

struct Waiting {
    caller: CallRef,
    remaining: usize,
}

pub struct Host {
    data_dir: PathBuf,
    extensions: HashMap<String, Extension>,
    storage: HashMap<(String, String), StorageArea>,
    sessions: HashMap<(TabId, String), Session>,
    waiting: HashMap<u64, Waiting>,
    next_wait: u64,
}

fn reply_script(session: &Session, message: &Value) -> String {
    format!(
        "window[{}]({}, {});",
        crate::script::value_for_js(&Value::String(session.entry.clone())),
        crate::script::value_for_js(&Value::String(session.token.clone())),
        crate::script::value_for_js(message)
    )
}

impl Host {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            extensions: HashMap::new(),
            storage: HashMap::new(),
            sessions: HashMap::new(),
            waiting: HashMap::new(),
            next_wait: 1,
        }
    }

    pub fn add(&mut self, extension: Extension) {
        self.extensions.insert(extension.uuid.clone(), extension);
    }

    pub fn remove(&mut self, uuid: &str) -> Option<Extension> {
        self.sessions.retain(|(_, u), _| u != uuid);
        self.storage.retain(|(u, _), _| u != uuid);
        self.extensions.remove(uuid)
    }

    pub fn extension(&self, uuid: &str) -> Option<&Extension> {
        self.extensions.get(uuid)
    }

    pub fn extensions(&self) -> impl Iterator<Item = &Extension> {
        self.extensions.values()
    }

    pub fn navigated(&mut self, tab: TabId) {
        self.sessions.retain(|(t, _), _| *t != tab);
    }

    pub fn has_page_session(&self, tab: TabId, uuid: &str) -> bool {
        self.sessions
            .get(&(tab, uuid.to_owned()))
            .is_some_and(|s| s.context == Context::Page)
    }

    fn area(&mut self, uuid: &str, area: &str) -> Option<&mut StorageArea> {
        let id = self.extensions.get(uuid)?.id.clone();
        let key = (uuid.to_owned(), area.to_owned());
        if !self.storage.contains_key(&key) {
            let path = match area {
                "session" => None,
                _ => Some(
                    self.data_dir
                        .join("browser-extension-data")
                        .join(sanitize(&id))
                        .join(if area == "sync" {
                            "storage-sync.js"
                        } else {
                            "storage.js"
                        }),
                ),
            };
            self.storage.insert(key.clone(), StorageArea::open(path));
        }
        self.storage.get_mut(&key)
    }

    pub fn reply(&self, call: &CallRef, result: Result<Value, String>) -> Option<Outgoing> {
        let session = self.sessions.get(&(call.tab, call.uuid.clone()))?;
        let message = match result {
            Ok(Value::Null) => json!({ "type": "reply", "id": call.id }),
            Ok(value) => json!({ "type": "reply", "id": call.id, "result": value }),
            Err(error) => json!({ "type": "reply", "id": call.id, "error": error }),
        };
        Some(Outgoing::Eval {
            tab: call.tab,
            script: reply_script(session, &message),
        })
    }

    pub fn event(
        &self,
        uuid: Option<&str>,
        context: Option<Context>,
        tab: Option<TabId>,
        event: &str,
        args: Value,
    ) -> Vec<Outgoing> {
        let message = json!({ "type": "event", "event": event, "args": args });
        self.sessions
            .iter()
            .filter(|((t, u), s)| {
                uuid.is_none_or(|uuid| u == uuid)
                    && context.is_none_or(|c| s.context == c)
                    && tab.is_none_or(|tab| *t == tab)
            })
            .map(|((t, _), s)| Outgoing::Eval {
                tab: *t,
                script: reply_script(s, &message),
            })
            .collect()
    }

    pub fn handle_console(&mut self, tab: TabId, page: Option<&Url>, text: &str) -> Vec<Outgoing> {
        let Some(json) = text.strip_prefix(MARK) else {
            return Vec::new();
        };
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(json) else {
            return Vec::new();
        };
        let get = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default();
        let uuid = get("ext").to_owned();
        if !self.extensions.contains_key(&uuid) {
            return Vec::new();
        }
        let key = (tab, uuid.clone());
        if get("type") == "hello" {
            if self.sessions.contains_key(&key) {
                return Vec::new();
            }
            let context = match get("context") {
                "page" => Context::Page,
                _ => Context::Content,
            };
            let allowed = match context {
                Context::Page => page.is_some_and(|p| {
                    p.scheme() == "moz-extension" && p.host_str() == Some(uuid.as_str())
                }),
                Context::Content => {
                    page.is_some_and(|p| matches!(p.scheme(), "http" | "https" | "file"))
                }
            };
            let token = get("token");
            let entry = get("entry");
            if !allowed || token.len() < 32 || !entry.starts_with("__klin_") {
                return Vec::new();
            }
            self.sessions.insert(
                key,
                Session {
                    token: token.to_owned(),
                    entry: entry.to_owned(),
                    context,
                    url: page.map(|p| p.to_string()).unwrap_or_default(),
                },
            );
            return Vec::new();
        }
        let Some(session) = self.sessions.get(&key).cloned() else {
            return Vec::new();
        };
        if get("token") != session.token {
            return Vec::new();
        }
        match get("type") {
            "call" => {
                let id = message.get("id").and_then(Value::as_u64).unwrap_or(0);
                let call = CallRef {
                    tab,
                    uuid: uuid.clone(),
                    id,
                };
                let api = get("api").to_owned();
                let args = match message.get("args") {
                    Some(Value::Array(args)) => args.clone(),
                    _ => Vec::new(),
                };
                self.call(call, &session, &api, args)
            }
            "respond" => {
                let call_id = message.get("callId").and_then(Value::as_u64).unwrap_or(0);
                self.respond(call_id, &message)
            }
            _ => Vec::new(),
        }
    }

    fn respond(&mut self, call_id: u64, message: &Map<String, Value>) -> Vec<Outgoing> {
        let Some(waiting) = self.waiting.get_mut(&call_id) else {
            return Vec::new();
        };
        let result = if let Some(error) = message.get("error") {
            Some(Err(error.as_str().unwrap_or("error").to_owned()))
        } else if message.get("none").is_some() {
            waiting.remaining = waiting.remaining.saturating_sub(1);
            (waiting.remaining == 0).then_some(Ok(Value::Null))
        } else {
            Some(Ok(message.get("value").cloned().unwrap_or(Value::Null)))
        };
        let Some(result) = result else {
            return Vec::new();
        };
        let waiting = self.waiting.remove(&call_id).expect("present");
        self.reply(&waiting.caller, result).into_iter().collect()
    }

    fn deliver_message(
        &mut self,
        call: CallRef,
        targets: Vec<(TabId, Session)>,
        message: Value,
        sender: Value,
    ) -> Vec<Outgoing> {
        if targets.is_empty() {
            return self
                .reply(
                    &call,
                    Err("Could not establish connection. Receiving end does not exist.".into()),
                )
                .into_iter()
                .collect();
        }
        let wait = self.next_wait;
        self.next_wait += 1;
        self.waiting.insert(
            wait,
            Waiting {
                caller: call,
                remaining: targets.len(),
            },
        );
        let event = json!({
            "type": "event",
            "event": "runtime.onMessage",
            "args": [message, sender],
            "callId": wait,
        });
        targets
            .into_iter()
            .map(|(tab, session)| Outgoing::Eval {
                tab,
                script: reply_script(&session, &event),
            })
            .collect()
    }

    fn call(
        &mut self,
        call: CallRef,
        session: &Session,
        api: &str,
        args: Vec<Value>,
    ) -> Vec<Outgoing> {
        let uuid = call.uuid.clone();
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
        match api {
            "storage.get" | "storage.set" | "storage.remove" | "storage.clear" => {
                let area_name = arg(0).as_str().unwrap_or("local").to_owned();
                if !matches!(area_name.as_str(), "local" | "sync" | "session") {
                    return self
                        .reply(&call, Err("unknown storage area".into()))
                        .into_iter()
                        .collect();
                }
                if session.context == Context::Content && area_name == "session" {
                    return self
                        .reply(
                            &call,
                            Err("storage.session is not available to content scripts".into()),
                        )
                        .into_iter()
                        .collect();
                }
                let Some(area) = self.area(&uuid, &area_name) else {
                    return Vec::new();
                };
                let (result, changes) = match api {
                    "storage.get" => (area.get(&arg(1)), None),
                    "storage.set" => (Value::Null, Some(area.set(&arg(1)))),
                    "storage.remove" => (Value::Null, Some(area.remove(&arg(1)))),
                    _ => (Value::Null, Some(area.clear())),
                };
                let mut out: Vec<Outgoing> = self.reply(&call, Ok(result)).into_iter().collect();
                if let Some(changes) = changes.filter(|c| !c.is_empty()) {
                    let changes = Value::Object(changes);
                    out.extend(self.event(
                        Some(&uuid),
                        None,
                        None,
                        "storage.onChanged",
                        json!([changes.clone(), area_name]),
                    ));
                    out.extend(self.event(
                        Some(&uuid),
                        None,
                        None,
                        &format!("storage.{area_name}.onChanged"),
                        json!([changes]),
                    ));
                }
                out
            }
            "runtime.sendMessage" => {
                let targets: Vec<(TabId, Session)> = self
                    .sessions
                    .iter()
                    .filter(|((t, u), s)| {
                        *u == uuid && s.context == Context::Page && *t != call.tab
                    })
                    .map(|((t, _), s)| (*t, s.clone()))
                    .collect();
                let id = self
                    .extensions
                    .get(&uuid)
                    .map(|e| e.id.clone())
                    .unwrap_or_default();
                let mut sender = json!({ "id": id, "url": session.url });
                if session.context == Context::Content {
                    sender["tab"] = json!({ "id": call.tab.get(), "url": session.url });
                    sender["frameId"] = json!(0);
                }
                self.deliver_message(call, targets, arg(0), sender)
            }
            "tabs.sendMessage" if session.context == Context::Page => {
                let Some(target) = arg(0).as_u64().map(TabId::from_raw) else {
                    return self
                        .reply(&call, Err("Invalid tab ID".into()))
                        .into_iter()
                        .collect();
                };
                let targets: Vec<(TabId, Session)> = self
                    .sessions
                    .get(&(target, uuid.clone()))
                    .filter(|s| s.context == Context::Content)
                    .map(|s| vec![(target, s.clone())])
                    .unwrap_or_default();
                let id = self
                    .extensions
                    .get(&uuid)
                    .map(|e| e.id.clone())
                    .unwrap_or_default();
                let sender = json!({ "id": id, "url": session.url });
                self.deliver_message(call, targets, arg(1), sender)
            }
            _ if session.context == Context::Page || api.starts_with("runtime.") => {
                let extension_id = self
                    .extensions
                    .get(&uuid)
                    .map(|e| e.id.clone())
                    .unwrap_or_default();
                vec![Outgoing::Api {
                    call,
                    extension_id,
                    api: api.to_owned(),
                    args,
                }]
            }
            _ => self
                .reply(&call, Err(format!("{api} is not available here")))
                .into_iter()
                .collect(),
        }
    }
}

pub fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "@.-_{}".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const UUID: &str = "11111111-2222-4333-8444-555555555555";

    fn host() -> (tempfile::TempDir, Host) {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join("ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"manifest_version": 2, "name": "T", "version": "1"}"#,
        )
        .unwrap();
        let package = Package::open(&ext_dir).unwrap();
        let (manifest, _) = Manifest::load(&package, "ru").unwrap();
        let mut host = Host::new(dir.path().join("profile"));
        host.add(Extension {
            id: "t@example.com".into(),
            uuid: UUID.into(),
            manifest,
            package,
        });
        (dir, host)
    }

    fn msg(value: Value) -> String {
        format!("{MARK}{value}")
    }

    fn hello(host: &mut Host, tab: TabId, url: &str, context: &str, token: &str) {
        let url = Url::parse(url).unwrap();
        host.handle_console(
            tab,
            Some(&url),
            &msg(json!({"type": "hello", "ext": UUID, "token": token, "entry": "__klin_e", "context": context})),
        );
    }

    fn call(
        host: &mut Host,
        tab: TabId,
        url: &str,
        token: &str,
        id: u64,
        api: &str,
        args: Value,
    ) -> Vec<Outgoing> {
        let url = Url::parse(url).unwrap();
        host.handle_console(
            tab,
            Some(&url),
            &msg(json!({"type": "call", "ext": UUID, "token": token, "id": id, "api": api, "args": args})),
        )
    }

    fn scripts(out: &[Outgoing]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                Outgoing::Eval { script, .. } => Some(script.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn checks_sessions_tokens_and_origins() {
        let (_dir, mut host) = host();
        let tab = TabId::next();
        hello(&mut host, tab, "https://evil.example/", "page", TOKEN);
        assert!(!host.has_page_session(tab, UUID));
        let page = format!("moz-extension://{UUID}/popup.html");
        hello(&mut host, tab, &page, "page", TOKEN);
        assert!(host.has_page_session(tab, UUID));
        hello(
            &mut host,
            tab,
            &page,
            "page",
            "ffffffffffffffffffffffffffffffff",
        );
        assert!(
            call(
                &mut host,
                tab,
                &page,
                "ffffffffffffffffffffffffffffffff",
                1,
                "storage.get",
                json!(["local", null])
            )
            .is_empty()
        );
        let out = call(
            &mut host,
            tab,
            &page,
            TOKEN,
            1,
            "storage.get",
            json!(["local", null]),
        );
        let script = &scripts(&out)[0];
        let prefix = format!("window[\"__klin_e\"](\"{TOKEN}\", ");
        let body: Value = serde_json::from_str(
            script
                .strip_prefix(&prefix)
                .unwrap()
                .strip_suffix(");")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body, json!({"type": "reply", "id": 1, "result": {}}));
        host.navigated(tab);
        assert!(
            call(
                &mut host,
                tab,
                &page,
                TOKEN,
                2,
                "storage.get",
                json!(["local", null])
            )
            .is_empty()
        );
    }

    #[test]
    fn stores_data_and_notifies_listeners() {
        let (dir, mut host) = host();
        let content = TabId::next();
        let background = TabId::next();
        hello(
            &mut host,
            content,
            "https://site.example/",
            "content",
            TOKEN,
        );
        let bg_url = format!("moz-extension://{UUID}/{BACKGROUND_PAGE}");
        hello(&mut host, background, &bg_url, "page", TOKEN);
        let out = call(
            &mut host,
            content,
            "https://site.example/",
            TOKEN,
            1,
            "storage.set",
            json!(["local", {"k": 1}]),
        );
        let s = scripts(&out);
        assert_eq!(s.len(), 5);
        assert!(s.iter().filter(|s| s.contains("storage.onChanged")).count() == 2);
        assert!(
            dir.path()
                .join("profile/browser-extension-data/t@example.com/storage.js")
                .exists()
        );
        let denied = call(
            &mut host,
            content,
            "https://site.example/",
            TOKEN,
            2,
            "storage.get",
            json!(["session", null]),
        );
        assert!(scripts(&denied)[0].contains("not available"));
        let tabs = call(
            &mut host,
            content,
            "https://site.example/",
            TOKEN,
            3,
            "tabs.query",
            json!([{}]),
        );
        assert!(scripts(&tabs)[0].contains("not available"));
        let api = call(
            &mut host,
            background,
            &bg_url,
            TOKEN,
            4,
            "tabs.query",
            json!([{"active": true}]),
        );
        assert!(matches!(&api[0], Outgoing::Api { api, .. } if api == "tabs.query"));
    }

    #[test]
    fn routes_messages_between_content_scripts_and_pages() {
        let (_dir, mut host) = host();
        let content = TabId::next();
        let background = TabId::next();
        hello(
            &mut host,
            content,
            "https://site.example/a",
            "content",
            TOKEN,
        );
        let none = call(
            &mut host,
            content,
            "https://site.example/a",
            TOKEN,
            1,
            "runtime.sendMessage",
            json!([{"q": 1}]),
        );
        assert!(scripts(&none)[0].contains("Receiving end does not exist"));
        let bg_url = format!("moz-extension://{UUID}/{BACKGROUND_PAGE}");
        hello(&mut host, background, &bg_url, "page", TOKEN);
        let out = call(
            &mut host,
            content,
            "https://site.example/a",
            TOKEN,
            2,
            "runtime.sendMessage",
            json!([{"q": 1}]),
        );
        let script = &scripts(&out)[0];
        assert!(script.contains("runtime.onMessage"));
        assert!(script.contains(&format!("\"tab\":{{\"id\":{}", content.get())));
        let url = Url::parse(&bg_url).unwrap();
        let reply = host.handle_console(
            background,
            Some(&url),
            &msg(json!({"type": "respond", "ext": UUID, "token": TOKEN, "callId": 1, "value": {"a": 2}})),
        );
        assert!(
            matches!(&reply[0], Outgoing::Eval { tab, script } if *tab == content && script.contains("\"result\":{\"a\":2}"))
        );
        let back = call(
            &mut host,
            background,
            &bg_url,
            TOKEN,
            3,
            "tabs.sendMessage",
            json!([content.get(), "ping"]),
        );
        assert!(matches!(&back[0], Outgoing::Eval { tab, .. } if *tab == content));
        let reply = host.handle_console(
            content,
            Some(&Url::parse("https://site.example/a").unwrap()),
            &msg(
                json!({"type": "respond", "ext": UUID, "token": TOKEN, "callId": 2, "none": true}),
            ),
        );
        assert!(
            matches!(&reply[0], Outgoing::Eval { tab, script } if *tab == background && script.contains("\"id\":3"))
        );
    }
}
