use std::collections::HashMap;
use std::path::PathBuf;

use klin_shared::{TabId, Url};
use serde_json::{Map, Value, json};

use crate::dnr::{self, RuleSet};
use crate::manifest::Manifest;
use crate::match_pattern::MatchPattern;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Continue,
    Cancel,
    Redirect(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestEvent {
    pub id: u64,
    pub url: Url,
    pub method: String,
    pub kind: String,
    pub tab: Option<TabId>,
    pub document: Option<Url>,
    pub redirect: bool,
}

struct Listener {
    uuid: String,
    tab: TabId,
    id: String,
    event: String,
    urls: Vec<MatchPattern>,
    types: Option<Vec<String>>,
    tab_filter: Option<i64>,
    blocking: bool,
}

#[derive(Default)]
struct DnrState {
    statics: Vec<(String, bool, RuleSet)>,
    dynamic: RuleSet,
    session: RuleSet,
}

fn base_domain(host: &str) -> &str {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    psl::domain_str(host).unwrap_or(host)
}

struct Port {
    uuid: String,
    opener: TabId,
    others: Vec<TabId>,
}

struct DynamicScript {
    uuid: String,
    owner: TabId,
    id: String,
    spec: crate::script::ScriptSpec,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Decide {
        request: u64,
        decision: Decision,
    },
    EvalWithResult {
        call: CallRef,
        tab: TabId,
        script: String,
    },
    ScriptsChanged,
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

enum Waiting {
    Message {
        caller: CallRef,
        remaining: usize,
    },
    Request {
        request: u64,
        remaining: usize,
        redirect: Option<String>,
    },
}

pub struct Host {
    data_dir: PathBuf,
    extensions: HashMap<String, Extension>,
    storage: HashMap<(String, String), StorageArea>,
    sessions: HashMap<(TabId, String), Session>,
    waiting: HashMap<u64, Waiting>,
    next_wait: u64,
    listeners: Vec<Listener>,
    dnr: HashMap<String, DnrState>,
    ports: HashMap<String, Port>,
    dynamic_scripts: Vec<DynamicScript>,
    active_tab: Option<TabId>,
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
            listeners: Vec::new(),
            dnr: HashMap::new(),
            ports: HashMap::new(),
            dynamic_scripts: Vec::new(),
            active_tab: None,
        }
    }

    pub fn add(&mut self, extension: Extension) {
        let state = self.load_dnr(&extension);
        if state.statics.iter().any(|(_, _, s)| !s.rules.is_empty())
            || !state.dynamic.rules.is_empty()
            || extension.manifest.has_permission("declarativeNetRequest")
            || extension
                .manifest
                .has_permission("declarativeNetRequestWithHostAccess")
        {
            self.dnr.insert(extension.uuid.clone(), state);
        }
        self.extensions.insert(extension.uuid.clone(), extension);
    }

    pub fn remove(&mut self, uuid: &str) -> Option<Extension> {
        self.sessions.retain(|(_, u), _| u != uuid);
        self.ports.retain(|_, p| p.uuid != uuid);
        self.dynamic_scripts.retain(|d| d.uuid != uuid);
        self.storage.retain(|(u, _), _| u != uuid);
        self.listeners.retain(|l| l.uuid != uuid);
        self.dnr.remove(uuid);
        self.extensions.remove(uuid)
    }

    pub fn wants_requests(&self) -> bool {
        !self.dnr.is_empty()
            || self.extensions.values().any(|e| {
                e.manifest.has_permission("webRequest")
                    || e.manifest.has_permission("webRequestBlocking")
            })
    }

    fn dnr_path(&self, extension_id: &str) -> PathBuf {
        self.data_dir
            .join("browser-extension-data")
            .join(sanitize(extension_id))
            .join("dnr.json")
    }

    fn load_dnr(&self, extension: &Extension) -> DnrState {
        let saved: Value = std::fs::read_to_string(self.dnr_path(&extension.id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let enabled_override: Option<Vec<String>> = saved
            .get("enabledRulesets")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            });
        let mut state = DnrState::default();
        for resource in extension
            .manifest
            .raw
            .get("declarative_net_request")
            .and_then(|d| d.get("rule_resources"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(id), Some(path)) = (
                resource.get("id").and_then(Value::as_str),
                resource.get("path").and_then(Value::as_str),
            ) else {
                continue;
            };
            let enabled = match &enabled_override {
                Some(list) => list.iter().any(|e| e == id),
                None => resource
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            let rules = extension
                .package
                .read_text(path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|v| RuleSet::parse(&v))
                .unwrap_or_else(|| Err(format!("cannot read {path}")));
            match rules {
                Ok(rules) => state.statics.push((id.to_owned(), enabled, rules)),
                Err(error) => log::warn!("{}: ruleset {id}: {error}", extension.id),
            }
        }
        if let Some(dynamic) = saved.get("dynamicRules") {
            match RuleSet::parse(dynamic) {
                Ok(rules) => state.dynamic = rules,
                Err(error) => log::warn!("{}: dynamic rules: {error}", extension.id),
            }
        }
        state
    }

    fn save_dnr(&self, uuid: &str) {
        let (Some(extension), Some(state)) = (self.extensions.get(uuid), self.dnr.get(uuid)) else {
            return;
        };
        let enabled: Vec<&str> = state
            .statics
            .iter()
            .filter(|(_, on, _)| *on)
            .map(|(id, _, _)| id.as_str())
            .collect();
        let text = json!({ "dynamicRules": state.dynamic.to_json(), "enabledRulesets": enabled })
            .to_string();
        let path = self.dnr_path(&extension.id);
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, text));
        if let Err(error) = result {
            log::error!("{}: {error}", path.display());
        }
    }

    fn request_details(&self, request: &RequestEvent) -> Value {
        let third_party = match (
            request.url.host_str(),
            request.document.as_ref().and_then(|d| d.host_str()),
        ) {
            (Some(a), Some(b)) => base_domain(a) != base_domain(b),
            _ => false,
        };
        let main = request.kind == "main_frame";
        let mut details = json!({
            "requestId": request.id.to_string(),
            "url": request.url.as_str(),
            "method": request.method,
            "type": request.kind,
            "tabId": request.tab.map_or(-1, |t| t.get() as i64),
            "frameId": 0,
            "parentFrameId": if main { -1 } else { 0 },
            "timeStamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64() * 1000.0),
            "thirdParty": third_party,
            "incognito": false,
            "cookieStoreId": "firefox-default",
            "frameAncestors": [],
            "urlClassification": { "firstParty": [], "thirdParty": [] },
            "proxyInfo": null,
            "ip": null,
        });
        if let Some(document) = &request.document {
            details["documentUrl"] = json!(document.as_str());
            details["originUrl"] = json!(document.as_str());
        }
        details
    }

    pub fn before_request(&mut self, request: &RequestEvent) -> (Option<Decision>, Vec<Outgoing>) {
        if !matches!(request.url.scheme(), "http" | "https" | "ws" | "wss") {
            return (Some(Decision::Continue), Vec::new());
        }
        let third_party = match (
            request.url.host_str(),
            request.document.as_ref().and_then(|d| d.host_str()),
        ) {
            (Some(a), Some(b)) => base_domain(a) != base_domain(b),
            _ => false,
        };
        let dnr_request = dnr::Request {
            url: &request.url,
            initiator: request.document.as_ref(),
            kind: &request.kind,
            method: &request.method,
            tab_id: request.tab.map_or(-1, |t| t.get() as i64),
            third_party,
        };
        let mut redirect = None;
        for (uuid, state) in &self.dnr {
            let sets = state
                .statics
                .iter()
                .filter(|(_, on, _)| *on)
                .map(|(_, _, s)| s)
                .chain([&state.dynamic, &state.session]);
            match dnr::evaluate(sets, &dnr_request, uuid) {
                Some(dnr::Outcome::Block) => return (Some(Decision::Cancel), Vec::new()),
                Some(dnr::Outcome::Redirect(url)) => redirect = redirect.or(Some(url)),
                _ => {}
            }
        }
        let details = self.request_details(request);
        let tab_id = request.tab.map_or(-1, |t| t.get() as i64);
        let mut out = Vec::new();
        let mut blocking = Vec::new();
        for listener in &self.listeners {
            if listener.event != "onBeforeRequest" {
                continue;
            }
            let Some(extension) = self.extensions.get(&listener.uuid) else {
                continue;
            };
            let permitted = extension
                .manifest
                .host_permissions
                .iter()
                .any(|p| p.matches(&request.url));
            if !permitted
                || !listener.urls.iter().any(|p| p.matches(&request.url))
                || listener
                    .types
                    .as_ref()
                    .is_some_and(|t| !t.contains(&request.kind))
                || listener.tab_filter.is_some_and(|t| t != tab_id)
            {
                continue;
            }
            let Some(session) = self.sessions.get(&(listener.tab, listener.uuid.clone())) else {
                continue;
            };
            if listener.blocking {
                blocking.push((listener.tab, session.clone(), listener.id.clone()));
            } else {
                let event = json!({
                    "type": "event",
                    "event": format!("webRequest:{}", listener.id),
                    "args": [details],
                });
                out.push(Outgoing::Eval {
                    tab: listener.tab,
                    script: reply_script(session, &event),
                });
            }
        }
        log::debug!(
            target: "extensions",
            "request {}: {} listeners, {} blocking",
            request.id,
            self.listeners.len(),
            blocking.len()
        );
        if blocking.is_empty() {
            let decision = redirect.map_or(Decision::Continue, Decision::Redirect);
            return (Some(decision), out);
        }
        let wait = self.next_wait;
        self.next_wait += 1;
        self.waiting.insert(
            wait,
            Waiting::Request {
                request: request.id,
                remaining: blocking.len(),
                redirect,
            },
        );
        for (tab, session, id) in blocking {
            let event = json!({
                "type": "event",
                "event": format!("webRequest:{id}"),
                "args": [details],
                "callId": wait,
                "single": true,
            });
            out.push(Outgoing::Eval {
                tab,
                script: reply_script(&session, &event),
            });
        }
        (None, out)
    }

    pub fn extension(&self, uuid: &str) -> Option<&Extension> {
        self.extensions.get(uuid)
    }

    pub fn extensions(&self) -> impl Iterator<Item = &Extension> {
        self.extensions.values()
    }

    pub fn navigated(&mut self, tab: TabId) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let closed: Vec<String> = self
            .ports
            .iter()
            .filter(|(_, p)| p.opener == tab || p.others.contains(&tab))
            .map(|(id, _)| id.clone())
            .collect();
        for id in closed {
            out.extend(self.close_port(&id, Some(tab)));
        }
        self.sessions.retain(|(t, _), _| *t != tab);
        self.listeners.retain(|l| l.tab != tab);
        let before = self.dynamic_scripts.len();
        self.dynamic_scripts.retain(|d| d.owner != tab);
        if self.dynamic_scripts.len() != before {
            out.push(Outgoing::ScriptsChanged);
        }
        out
    }

    pub fn dynamic_scripts(&self, uuid: &str) -> Vec<crate::script::ScriptSpec> {
        self.dynamic_scripts
            .iter()
            .filter(|d| d.uuid == uuid)
            .map(|d| d.spec.clone())
            .collect()
    }

    fn port_message(&self, tab: TabId, uuid: &str, message: Value) -> Option<Outgoing> {
        let session = self.sessions.get(&(tab, uuid.to_owned()))?;
        Some(Outgoing::Eval {
            tab,
            script: reply_script(session, &message),
        })
    }

    fn close_port(&mut self, id: &str, from: Option<TabId>) -> Vec<Outgoing> {
        let Some(port) = self.ports.remove(id) else {
            return Vec::new();
        };
        let message = json!({ "type": "port", "action": "disconnect", "port": id });
        std::iter::once(port.opener)
            .chain(port.others.iter().copied())
            .filter(|t| Some(*t) != from)
            .filter_map(|t| self.port_message(t, &port.uuid, message.clone()))
            .collect()
    }

    fn handle_port(
        &mut self,
        tab: TabId,
        uuid: &str,
        session: &Session,
        message: &Map<String, Value>,
    ) -> Vec<Outgoing> {
        let get = |key: &str| {
            message
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let id = get("port");
        if id.len() < 16 {
            return Vec::new();
        }
        match get("action").as_str() {
            "connect" => {
                if self.ports.contains_key(&id) {
                    return Vec::new();
                }
                let target_tab = message
                    .get("tabId")
                    .and_then(Value::as_u64)
                    .map(TabId::from_raw);
                let targets: Vec<TabId> = self
                    .sessions
                    .iter()
                    .filter(|((t, u), s)| {
                        u == uuid
                            && *t != tab
                            && match target_tab {
                                Some(target) => *t == target && s.context == Context::Content,
                                None => s.context == Context::Page,
                            }
                    })
                    .map(|((t, _), _)| *t)
                    .collect();
                if targets.is_empty() || (target_tab.is_some() && session.context != Context::Page)
                {
                    let message = json!({
                        "type": "port", "action": "disconnect", "port": id,
                        "error": "Could not establish connection. Receiving end does not exist.",
                    });
                    return self.port_message(tab, uuid, message).into_iter().collect();
                }
                let extension_id = self
                    .extensions
                    .get(uuid)
                    .map(|e| e.id.clone())
                    .unwrap_or_default();
                let mut sender = json!({ "id": extension_id, "url": session.url });
                if session.context == Context::Content {
                    sender["tab"] = json!({ "id": tab.get(), "url": session.url });
                    sender["frameId"] = json!(0);
                }
                let connect = json!({
                    "type": "port", "action": "connect", "port": id,
                    "name": get("name"), "sender": sender,
                });
                let out = targets
                    .iter()
                    .filter_map(|t| self.port_message(*t, uuid, connect.clone()))
                    .collect();
                self.ports.insert(
                    id,
                    Port {
                        uuid: uuid.to_owned(),
                        opener: tab,
                        others: targets,
                    },
                );
                out
            }
            "post" => {
                let Some(port) = self.ports.get(&id) else {
                    return Vec::new();
                };
                if port.uuid != uuid {
                    return Vec::new();
                }
                let receivers: Vec<TabId> = if port.opener == tab {
                    port.others.clone()
                } else if port.others.contains(&tab) {
                    vec![port.opener]
                } else {
                    return Vec::new();
                };
                let deliver = json!({
                    "type": "port", "action": "message", "port": id,
                    "message": message.get("message").cloned().unwrap_or(Value::Null),
                });
                receivers
                    .into_iter()
                    .filter_map(|t| self.port_message(t, uuid, deliver.clone()))
                    .collect()
            }
            "disconnect" => {
                let owned = self.ports.get(&id).is_some_and(|p| {
                    p.uuid == uuid && (p.opener == tab || p.others.contains(&tab))
                });
                if owned {
                    self.close_port(&id, Some(tab))
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn content_session(&self, tab: TabId, uuid: &str) -> Option<&Session> {
        self.sessions
            .get(&(tab, uuid.to_owned()))
            .filter(|s| s.context == Context::Content)
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
            Err(error) => {
                log::info!(target: "extensions", "call {} failed: {error}", call.id);
                json!({ "type": "reply", "id": call.id, "error": error })
            }
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
            "port" => {
                log::debug!(
                    target: "extensions",
                    "{tab} port {} {}",
                    get("action"),
                    message.get("message").map(|m| m.to_string().chars().take(300).collect::<String>()).unwrap_or_default()
                );
                self.handle_port(tab, &uuid, &session, &message)
            }
            "call" => {
                let id = message.get("id").and_then(Value::as_u64).unwrap_or(0);
                let call = CallRef {
                    tab,
                    uuid: uuid.clone(),
                    id,
                };
                let api = get("api").to_owned();
                log::debug!(target: "extensions", "{tab} {api}");
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
        match self.waiting.get_mut(&call_id) {
            Some(Waiting::Message { remaining, .. }) => {
                let result = if let Some(error) = message.get("error") {
                    Some(Err(error.as_str().unwrap_or("error").to_owned()))
                } else if message.get("none").is_some() {
                    *remaining = remaining.saturating_sub(1);
                    (*remaining == 0).then_some(Ok(Value::Null))
                } else {
                    Some(Ok(message.get("value").cloned().unwrap_or(Value::Null)))
                };
                let Some(result) = result else {
                    return Vec::new();
                };
                let Some(Waiting::Message { caller, .. }) = self.waiting.remove(&call_id) else {
                    return Vec::new();
                };
                self.reply(&caller, result).into_iter().collect()
            }
            Some(Waiting::Request {
                request,
                remaining,
                redirect,
            }) => {
                let value = message.get("value").cloned().unwrap_or(Value::Null);
                let request = *request;
                if value.get("cancel").and_then(Value::as_bool) == Some(true) {
                    self.waiting.remove(&call_id);
                    return vec![Outgoing::Decide {
                        request,
                        decision: Decision::Cancel,
                    }];
                }
                if redirect.is_none()
                    && let Some(url) = value.get("redirectUrl").and_then(Value::as_str)
                {
                    *redirect = Some(url.to_owned());
                }
                *remaining = remaining.saturating_sub(1);
                if *remaining > 0 {
                    return Vec::new();
                }
                let Some(Waiting::Request { redirect, .. }) = self.waiting.remove(&call_id) else {
                    return Vec::new();
                };
                vec![Outgoing::Decide {
                    request,
                    decision: redirect.map_or(Decision::Continue, Decision::Redirect),
                }]
            }
            None => Vec::new(),
        }
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
            Waiting::Message {
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
            "webRequest.addListener"
            | "webRequest.removeListener"
            | "webRequest.handlerBehaviorChanged"
                if session.context == Context::Page =>
            {
                if api == "webRequest.addListener" {
                    let filter = arg(2);
                    let urls: Vec<MatchPattern> = filter
                        .get("urls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter_map(|p| MatchPattern::parse(p).ok())
                        .collect();

                    let blocking = arg(3)
                        .as_array()
                        .is_some_and(|l| l.iter().any(|v| v.as_str() == Some("blocking")))
                        && self
                            .extensions
                            .get(&uuid)
                            .is_some_and(|e| e.manifest.has_permission("webRequestBlocking"));
                    self.listeners.push(Listener {
                        uuid: uuid.clone(),
                        tab: call.tab,
                        id: arg(1).as_str().unwrap_or_default().to_owned(),
                        event: arg(0).as_str().unwrap_or_default().to_owned(),
                        urls,
                        types: filter.get("types").and_then(Value::as_array).map(|l| {
                            l.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        }),
                        tab_filter: filter.get("tabId").and_then(Value::as_i64),
                        blocking,
                    });
                } else if api == "webRequest.removeListener" {
                    let id = arg(1).as_str().unwrap_or_default().to_owned();
                    self.listeners.retain(|l| !(l.uuid == uuid && l.id == id));
                }
                self.reply(&call, Ok(Value::Null)).into_iter().collect()
            }
            "tabs.executeScript" | "tabs.insertCSS" | "tabs.removeCSS"
                if session.context == Context::Page =>
            {
                self.tab_content_call(call, api, arg(0), arg(1))
            }
            "contentScripts.register" if session.context == Context::Page => {
                let result = self.register_script(&uuid, call.tab, &arg(0));
                let changed = result.is_ok();
                let mut out: Vec<Outgoing> = self.reply(&call, result).into_iter().collect();
                if changed {
                    out.push(Outgoing::ScriptsChanged);
                }
                out
            }
            "contentScripts.unregister" if session.context == Context::Page => {
                let id = arg(0).as_str().unwrap_or_default().to_owned();
                self.dynamic_scripts
                    .retain(|d| !(d.uuid == uuid && d.id == id));
                let mut out: Vec<Outgoing> =
                    self.reply(&call, Ok(Value::Null)).into_iter().collect();
                out.push(Outgoing::ScriptsChanged);
                out
            }
            _ if api.starts_with("declarativeNetRequest.") && session.context == Context::Page => {
                let result = self.dnr_api(&uuid, api, arg(0));
                self.reply(&call, result).into_iter().collect()
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

impl Host {
    fn read_source(&self, uuid: &str, value: &Value) -> Result<String, String> {
        if let Some(code) = value.get("code").and_then(Value::as_str) {
            return Ok(code.to_owned());
        }
        let file = value
            .get("file")
            .and_then(Value::as_str)
            .ok_or("Either code or file must be specified")?;
        let extension = self.extensions.get(uuid).ok_or("Unknown extension")?;
        extension.package.read_text(file).map_err(|e| e.to_string())
    }

    fn register_script(
        &mut self,
        uuid: &str,
        owner: TabId,
        options: &Value,
    ) -> Result<Value, String> {
        let patterns = |key: &str| -> Result<Vec<MatchPattern>, String> {
            options
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|p| MatchPattern::parse(p).map_err(|e| e.to_string()))
                .collect()
        };
        let strings = |key: &str| -> Vec<String> {
            options
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        let sources = |key: &str| -> Result<Vec<String>, String> {
            options
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|v| self.read_source(uuid, v))
                .collect()
        };
        let matches = patterns("matches")?;
        if matches.is_empty() {
            return Err("matches must not be empty".into());
        }
        let spec = crate::script::ScriptSpec {
            matches,
            exclude_matches: patterns("excludeMatches")?,
            include_globs: strings("includeGlobs"),
            exclude_globs: strings("excludeGlobs"),
            run_at: match options.get("runAt").and_then(Value::as_str) {
                Some("document_start") => crate::manifest::RunAt::DocumentStart,
                Some("document_end") => crate::manifest::RunAt::DocumentEnd,
                _ => crate::manifest::RunAt::DocumentIdle,
            },
            js: sources("js")?,
            css: sources("css")?,
        };
        let id = format!("{}-{}", uuid, self.next_wait);
        self.next_wait += 1;
        self.dynamic_scripts.push(DynamicScript {
            uuid: uuid.to_owned(),
            owner,
            id: id.clone(),
            spec,
        });
        Ok(json!(id))
    }

    fn tab_content_call(
        &mut self,
        call: CallRef,
        api: &str,
        tab_arg: Value,
        details: Value,
    ) -> Vec<Outgoing> {
        let uuid = call.uuid.clone();
        let target = match tab_arg.as_u64() {
            Some(id) => TabId::from_raw(id),
            None => match self.active_tab {
                Some(tab) => tab,
                None => {
                    return self
                        .reply(&call, Err("No active tab".into()))
                        .into_iter()
                        .collect();
                }
            },
        };
        let Some(session) = self.content_session(target, &uuid).cloned() else {
            return self
                .reply(&call, Err("Missing host permission for the tab".into()))
                .into_iter()
                .collect();
        };
        let source = match self.read_source(&uuid, &details) {
            Ok(source) => source,
            Err(error) => return self.reply(&call, Err(error)).into_iter().collect(),
        };
        log::debug!(target: "extensions", "{api}: {} bytes: {}", source.len(), source.chars().take(400).collect::<String>());
        let entry = crate::script::value_for_js(&Value::String(session.entry.clone()));
        let token = crate::script::value_for_js(&Value::String(session.token.clone()));
        let script = if api == "tabs.executeScript" {
            format!(
                "{{ const __klin = window[{entry}]({token}, {{ type: \"api\" }}); const browser = __klin.browser; const chrome = __klin.chrome;\n{source}\n}}"
            )
        } else {
            let key = details
                .get("code")
                .and_then(Value::as_str)
                .map(|c| format!("code:{c}"))
                .or_else(|| {
                    details
                        .get("file")
                        .and_then(Value::as_str)
                        .map(|f| format!("file:{f}"))
                })
                .unwrap_or_default();
            let message = json!({
                "type": "css",
                "key": key,
                "css": source,
                "remove": api == "tabs.removeCSS",
            });
            format!(
                "window[{entry}]({token}, {}) === true",
                crate::script::value_for_js(&message)
            )
        };
        vec![Outgoing::EvalWithResult {
            call,
            tab: target,
            script,
        }]
    }

    pub fn finish_eval(
        &self,
        call: &CallRef,
        api_value: Result<Value, String>,
        execute: bool,
    ) -> Option<Outgoing> {
        let value = match api_value {
            Ok(value) if execute => Ok(json!([value])),
            Ok(_) => Ok(Value::Null),
            Err(error) => Err(error),
        };
        self.reply(call, value)
    }

    pub fn set_active_tab(&mut self, tab: Option<TabId>) {
        self.active_tab = tab;
    }

    fn dnr_api(&mut self, uuid: &str, api: &str, options: Value) -> Result<Value, String> {
        let permitted = self.extensions.get(uuid).is_some_and(|e| {
            e.manifest.has_permission("declarativeNetRequest")
                || e.manifest
                    .has_permission("declarativeNetRequestWithHostAccess")
        });
        if !permitted {
            return Err("Missing declarativeNetRequest permission".into());
        }
        let state = self.dnr.entry(uuid.to_owned()).or_default();
        let ids = |key: &str| -> std::collections::HashSet<i64> {
            options
                .get(key)
                .and_then(Value::as_array)
                .map(|l| l.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default()
        };
        let strings = |key: &str| -> Vec<String> {
            options
                .get(key)
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let filter_ids = ids("ruleIds");
        let mut save = false;
        let result = match api {
            "declarativeNetRequest.updateDynamicRules"
            | "declarativeNetRequest.updateSessionRules" => {
                let dynamic = api.ends_with("DynamicRules");
                let set = if dynamic {
                    &mut state.dynamic
                } else {
                    &mut state.session
                };
                let mut next = set.clone();
                next.remove(&ids("removeRuleIds"));
                let added = RuleSet::parse(options.get("addRules").unwrap_or(&json!([])))?;
                for rule in added.rules {
                    if next.rules.iter().any(|r| r.id == rule.id) {
                        return Err(format!("Duplicate rule ID: {}", rule.id));
                    }
                    next.rules.push(rule);
                }
                *set = next;
                save = dynamic;
                Value::Null
            }
            "declarativeNetRequest.getDynamicRules" | "declarativeNetRequest.getSessionRules" => {
                let set = if api.ends_with("DynamicRules") {
                    &state.dynamic
                } else {
                    &state.session
                };
                Value::Array(
                    set.rules
                        .iter()
                        .filter(|r| filter_ids.is_empty() || filter_ids.contains(&r.id))
                        .map(|r| r.raw.clone())
                        .collect(),
                )
            }
            "declarativeNetRequest.getEnabledRulesets" => json!(
                state
                    .statics
                    .iter()
                    .filter(|(_, on, _)| *on)
                    .map(|(id, _, _)| id.clone())
                    .collect::<Vec<_>>()
            ),
            "declarativeNetRequest.updateEnabledRulesets" => {
                let enable = strings("enableRulesetIds");
                let disable = strings("disableRulesetIds");
                for id in enable.iter().chain(&disable) {
                    if !state.statics.iter().any(|(s, _, _)| s == id) {
                        return Err(format!("Invalid ruleset id: {id}"));
                    }
                }
                for (id, on, _) in &mut state.statics {
                    if disable.contains(id) {
                        *on = false;
                    }
                    if enable.contains(id) {
                        *on = true;
                    }
                }
                save = true;
                Value::Null
            }
            "declarativeNetRequest.isRegexSupported" => {
                let regex = options
                    .get("regex")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if dnr::regex_supported(regex) {
                    json!({ "isSupported": true })
                } else {
                    json!({ "isSupported": false, "reason": "syntaxError" })
                }
            }
            "declarativeNetRequest.getAvailableStaticRuleCount" => json!(30000),
            _ => return Err(format!("{api} is not supported yet")),
        };
        if save {
            self.save_dnr(uuid);
        }
        Ok(result)
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
        let _ = host.navigated(tab);
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

    #[test]
    fn blocking_web_request_listeners_decide_requests() {
        let dir = tempfile::tempdir().unwrap();
        let ext_dir = dir.path().join("ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"manifest_version": 2, "name": "W", "version": "1", "permissions": ["webRequest", "webRequestBlocking", "*://*.example.com/*"]}"#,
        )
        .unwrap();
        let package = Package::open(&ext_dir).unwrap();
        let (manifest, _) = Manifest::load(&package, "ru").unwrap();
        let mut host = Host::new(dir.path().join("profile"));
        host.add(Extension {
            id: "w@example.com".into(),
            uuid: UUID.into(),
            manifest,
            package,
        });
        assert!(host.wants_requests());
        let background = TabId::next();
        let bg_url = format!("moz-extension://{UUID}/{BACKGROUND_PAGE}");
        hello(&mut host, background, &bg_url, "page", TOKEN);
        call(
            &mut host,
            background,
            &bg_url,
            TOKEN,
            1,
            "webRequest.addListener",
            json!(["onBeforeRequest", "L1", {"urls": ["<all_urls>"], "types": ["script"]}, ["blocking"]]),
        );
        let request = |id: u64, url: &str, kind: &str| RequestEvent {
            id,
            url: Url::parse(url).unwrap(),
            method: "GET".into(),
            kind: kind.into(),
            tab: Some(TabId::next()),
            document: Some(Url::parse("https://www.example.com/").unwrap()),
            redirect: false,
        };
        let (decision, out) =
            host.before_request(&request(10, "https://cdn.example.com/a.js", "script"));
        assert_eq!(decision, None);
        let script = &scripts(&out)[0];
        assert!(script.contains("webRequest:L1") && script.contains("\"single\":true"));
        let url = Url::parse(&bg_url).unwrap();
        let decided = host.handle_console(
            background,
            Some(&url),
            &msg(json!({"type": "respond", "ext": UUID, "token": TOKEN, "callId": 1, "value": {"cancel": true}})),
        );
        assert_eq!(
            decided,
            [Outgoing::Decide {
                request: 10,
                decision: Decision::Cancel
            }]
        );
        let (image, _) =
            host.before_request(&request(11, "https://cdn.example.com/a.png", "image"));
        assert_eq!(image, Some(Decision::Continue));
        let (other_host, _) = host.before_request(&request(12, "https://other.org/a.js", "script"));
        assert_eq!(other_host, Some(Decision::Continue));
        let (_, out) = host.before_request(&request(13, "https://cdn.example.com/b.js", "script"));
        assert_eq!(out.len(), 1);
        let redirected = host.handle_console(
            background,
            Some(&url),
            &msg(json!({"type": "respond", "ext": UUID, "token": TOKEN, "callId": 2, "value": {"redirectUrl": "moz-extension://x/noop.js"}})),
        );
        assert_eq!(
            redirected,
            [Outgoing::Decide {
                request: 13,
                decision: Decision::Redirect("moz-extension://x/noop.js".into())
            }]
        );
        let _ = host.navigated(background);
        let (after, _) =
            host.before_request(&request(14, "https://cdn.example.com/c.js", "script"));
        assert_eq!(after, Some(Decision::Continue));
    }
}
