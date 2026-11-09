use klin_core::Engine;
use klin_extensions::host::{CallRef, Decision, Outgoing};
use klin_extensions::match_pattern::{MatchPattern, glob_matches};
use klin_shared::{TabId, Url, WindowId};
use serde_json::{Value, json};
use winit::event_loop::ActiveEventLoop;

use crate::Running;

pub(crate) type ExecResult = (CallRef, Result<Value, String>, bool);

fn tab_ids(value: &Value) -> Vec<TabId> {
    match value {
        Value::Array(list) => list
            .iter()
            .filter_map(Value::as_u64)
            .map(TabId::from_raw)
            .collect(),
        Value::Number(n) => n.as_u64().map(TabId::from_raw).into_iter().collect(),
        _ => Vec::new(),
    }
}

impl Running {
    pub(crate) fn current_window(&self) -> Option<WindowId> {
        self.focused
            .filter(|w| self.browser.window(*w).is_some())
            .or_else(|| self.browser.windows().first().map(|w| w.id))
    }

    fn reveals_tab(&self, uuid: &str, url: Option<&Url>) -> bool {
        let Some(extension) = self.extensions.host.extension(uuid) else {
            return false;
        };
        let manifest = &extension.manifest;
        manifest.has_permission("tabs")
            || manifest.has_permission("activeTab")
            || url.is_some_and(|u| {
                (u.scheme() == "moz-extension" && u.host_str() == Some(uuid))
                    || manifest.host_permissions.iter().any(|p| p.matches(u))
            })
    }

    pub(crate) fn tab_info(&self, uuid: &str, tab: TabId) -> Option<Value> {
        let t = self.browser.tab(tab)?;
        let window = self.browser.window(t.window)?;
        let index = window.tabs.iter().position(|id| *id == tab)?;
        let active = window.selected_tab() == Some(tab);
        let mut info = json!({
            "id": tab.get(),
            "index": index,
            "windowId": t.window.get(),
            "active": active,
            "highlighted": active,
            "selected": active,
            "pinned": false,
            "discarded": !t.loaded,
            "incognito": window.private,
            "status": if t.load_status == klin_shared::LoadStatus::Complete { "complete" } else { "loading" },
            "width": 0,
            "height": 0,
        });
        if self.reveals_tab(uuid, t.url.as_ref()) {
            if let Some(url) = &t.url {
                info["url"] = json!(url.as_str());
            }
            info["title"] = json!(t.title);
        }
        Some(info)
    }

    pub(crate) fn sync_extension_tabs(&mut self) {
        let active = self
            .current_window()
            .and_then(|w| self.browser.window(w))
            .and_then(|w| w.selected_tab());
        self.extensions.host.set_active_tab(active);
        if self.extensions.host.extensions().next().is_none() {
            return;
        }
        let mut tabs = std::collections::HashMap::new();
        let mut selected = std::collections::HashMap::new();
        for window in self.browser.windows() {
            for tab in &window.tabs {
                tabs.insert(*tab, window.id);
            }
            if let Some(tab) = window.selected_tab() {
                selected.insert(window.id, tab);
            }
        }
        let created: Vec<TabId> = tabs
            .keys()
            .filter(|t| !self.known_tabs.contains_key(t))
            .copied()
            .collect();
        let removed: Vec<(TabId, WindowId)> = self
            .known_tabs
            .iter()
            .filter(|(t, _)| !tabs.contains_key(t))
            .map(|(t, w)| (*t, *w))
            .collect();
        let activated: Vec<(WindowId, TabId, Option<TabId>)> = selected
            .iter()
            .filter(|(w, t)| self.active_tabs.get(w) != Some(t))
            .map(|(w, t)| (*w, *t, self.active_tabs.get(w).copied()))
            .collect();
        self.known_tabs = tabs;
        self.active_tabs = selected;
        let uuids: Vec<String> = self
            .extensions
            .host
            .extensions()
            .map(|e| e.uuid.clone())
            .collect();
        for tab in created {
            for uuid in &uuids {
                if let Some(info) = self.tab_info(uuid, tab) {
                    let out = self.extensions.host.event(
                        Some(uuid),
                        Some(klin_extensions::host::Context::Page),
                        None,
                        "tabs.onCreated",
                        json!([info]),
                    );
                    self.eval_all(out);
                }
            }
        }
        for (tab, window) in removed {
            self.extension_event(
                None,
                "tabs.onRemoved",
                json!([tab.get(), { "windowId": window.get(), "isWindowClosing": self.browser.window(window).is_none() }]),
            );
        }
        for (window, tab, previous) in activated {
            let mut info = json!({ "tabId": tab.get(), "windowId": window.get() });
            if let Some(previous) = previous {
                info["previousTabId"] = json!(previous.get());
            }
            self.extension_event(None, "tabs.onActivated", json!([info]));
        }
    }

    fn eval_all(&mut self, out: Vec<Outgoing>) {
        for item in out {
            if let Outgoing::Eval { tab, script } = item {
                self.engine.evaluate(tab, script);
            }
        }
    }

    pub(crate) fn web_request(&mut self, info: klin_core::WebRequestInfo) {
        if info.kind == "main_frame"
            && let Some(tab) = info.tab
        {
            self.navigation_event("onBeforeNavigate", tab, Some(&info.url));
        }
        let tab = info.tab.filter(|t| self.browser.tab(*t).is_some());
        let event = klin_extensions::host::RequestEvent {
            id: info.id,
            url: info.url,
            method: info.method,
            kind: info.kind.to_owned(),
            tab,
            document: info.document,
            redirect: info.redirect,
        };
        log::debug!(target: "extensions", "request {} {} {}", info.id, info.kind, event.url);
        let (decision, out) = self.extensions.host.before_request(&event);
        for item in out {
            if let Outgoing::Eval { tab, script } = item {
                self.engine.evaluate(tab, script);
            }
        }
        match decision {
            Some(decision) => self.decide_request(info.id, info.kind == "main_frame", decision),
            None => {
                self.pending_requests
                    .insert(info.id, info.kind == "main_frame");
            }
        }
    }

    fn decide_request(&mut self, request: u64, main_frame: bool, decision: Decision) {
        log::debug!(target: "extensions", "request {request} -> {decision:?}");
        let decision = match decision {
            Decision::Continue => klin_core::RequestDecision::Continue,
            Decision::Cancel => klin_core::RequestDecision::Cancel,
            Decision::Redirect(target) => match Url::parse(&target) {
                Ok(url) if url.scheme() == "moz-extension" => {
                    match self.extensions.extension_file(&url) {
                        Some((mime, body)) => klin_core::RequestDecision::Respond {
                            mime: mime.to_owned(),
                            body,
                        },
                        None => klin_core::RequestDecision::Cancel,
                    }
                }
                Ok(url) if main_frame => klin_core::RequestDecision::Navigate(url),
                Ok(url) => {
                    log::info!(
                        "redirecting a subresource to {url} is not supported; loading it as is"
                    );
                    klin_core::RequestDecision::Continue
                }
                Err(_) => klin_core::RequestDecision::Continue,
            },
        };
        self.engine.resolve_request(request, decision);
    }

    pub(crate) fn extension_outgoing(&mut self, event_loop: &ActiveEventLoop, out: Vec<Outgoing>) {
        for item in out {
            match item {
                Outgoing::EvalWithResult { call, tab, script } => {
                    let execute = !script.ends_with("=== true");
                    let tx = self.exec_tx.clone();
                    let proxy = self.proxy.clone();
                    self.engine
                        .evaluate_with_result(tab, script, move |result| {
                            let _ = tx.send((call, result, execute));
                            let _ = proxy.send_event(crate::Wake);
                        });
                }
                Outgoing::ScriptsChanged => self.extensions.update_scripts(&mut self.engine),
                Outgoing::Decide { request, decision } => {
                    let main_frame = self.pending_requests.remove(&request).unwrap_or(false);
                    self.decide_request(request, main_frame, decision);
                }
                Outgoing::Eval { tab, script } => self.engine.evaluate(tab, script),
                Outgoing::Api {
                    call,
                    extension_id: _,
                    api,
                    args,
                } => {
                    let result = self.extension_api(event_loop, &call, &api, args);
                    if let Some(Outgoing::Eval { tab, script }) =
                        self.extensions.host.reply(&call, result)
                    {
                        self.engine.evaluate(tab, script);
                    }
                }
            }
        }
    }

    fn query_tabs(&self, uuid: &str, query: &Value) -> Vec<Value> {
        let current = self.current_window();
        let get_bool = |key: &str| query.get(key).and_then(Value::as_bool);
        let patterns: Vec<MatchPattern> = match query.get("url") {
            Some(Value::String(p)) => MatchPattern::parse(p).into_iter().collect(),
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|p| MatchPattern::parse(p).ok())
                .collect(),
            _ => Vec::new(),
        };
        let has_url_filter = query.get("url").is_some();
        let mut result = Vec::new();
        for window in self.browser.windows() {
            let in_current = Some(window.id) == current;
            if (get_bool("currentWindow") == Some(true)
                || get_bool("lastFocusedWindow") == Some(true))
                && !in_current
            {
                continue;
            }
            if get_bool("currentWindow") == Some(false) && in_current {
                continue;
            }
            if let Some(id) = query.get("windowId").and_then(Value::as_i64)
                && id >= 0
                && id as u64 != window.id.get()
            {
                continue;
            }
            for (index, tab) in window.tabs.iter().enumerate() {
                let Some(t) = self.browser.tab(*tab) else {
                    continue;
                };
                let active = window.selected_tab() == Some(*tab);
                if get_bool("active").is_some_and(|a| a != active)
                    || get_bool("highlighted").is_some_and(|a| a != active)
                    || get_bool("pinned") == Some(true)
                    || query
                        .get("index")
                        .and_then(Value::as_u64)
                        .is_some_and(|i| i as usize != index)
                {
                    continue;
                }
                if let Some(status) = query.get("status").and_then(Value::as_str) {
                    let complete = t.load_status == klin_shared::LoadStatus::Complete;
                    if (status == "complete") != complete {
                        continue;
                    }
                }
                if has_url_filter
                    && (!self.reveals_tab(uuid, t.url.as_ref())
                        || !t
                            .url
                            .as_ref()
                            .is_some_and(|u| patterns.iter().any(|p| p.matches(u))))
                {
                    continue;
                }
                if let Some(title) = query.get("title").and_then(Value::as_str)
                    && !glob_matches(title, &t.title)
                {
                    continue;
                }
                if let Some(info) = self.tab_info(uuid, *tab) {
                    result.push(info);
                }
            }
        }
        result
    }

    fn extension_api(
        &mut self,
        event_loop: &ActiveEventLoop,
        call: &CallRef,
        api: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        let uuid = call.uuid.clone();
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
        let missing = || format!("Invalid tab ID: {}", arg(0));
        match api {
            "tabs.query" => Ok(Value::Array(self.query_tabs(&uuid, &arg(0)))),
            "tabs.get" => {
                let tab = tab_ids(&arg(0)).into_iter().next().ok_or_else(missing)?;
                self.tab_info(&uuid, tab).ok_or_else(missing)
            }
            "tabs.getCurrent" => Ok(self.tab_info(&uuid, call.tab).unwrap_or(Value::Null)),
            "tabs.create" => {
                let props = arg(0);
                let url = match props.get("url").and_then(Value::as_str) {
                    Some(url) => self
                        .extensions
                        .resolve(&uuid, url)
                        .ok_or_else(|| format!("Illegal URL: {url}"))?,
                    None => Url::parse(klin_core::prefs::NEW_TAB_URL).map_err(|e| e.to_string())?,
                };
                if matches!(url.scheme(), "javascript" | "data" | "chrome" | "file") {
                    return Err(format!("Illegal URL: {url}"));
                }
                let window = props
                    .get("windowId")
                    .and_then(Value::as_u64)
                    .map(WindowId::from_raw)
                    .filter(|w| self.browser.window(*w).is_some())
                    .or_else(|| self.current_window())
                    .ok_or("No window")?;
                let active = props.get("active").and_then(Value::as_bool).unwrap_or(true);
                let tab = self
                    .browser
                    .open_tab(&mut self.engine, window, url, None, active)
                    .ok_or("Could not open a tab")?;
                self.request_redraw(window);
                Ok(self.tab_info(&uuid, tab).unwrap_or(Value::Null))
            }
            "tabs.update" => {
                let tab = tab_ids(&arg(0))
                    .into_iter()
                    .next()
                    .or_else(|| {
                        self.current_window()
                            .and_then(|w| self.browser.window(w))
                            .and_then(|w| w.selected_tab())
                    })
                    .ok_or_else(missing)?;
                let props = arg(1);
                if let Some(url) = props.get("url").and_then(Value::as_str) {
                    let url = self
                        .extensions
                        .resolve(&uuid, url)
                        .ok_or_else(|| format!("Illegal URL: {url}"))?;
                    if matches!(url.scheme(), "javascript" | "data" | "chrome" | "file") {
                        return Err(format!("Illegal URL: {url}"));
                    }
                    self.engine.load(tab, &url);
                }
                if props.get("active").and_then(Value::as_bool) == Some(true) {
                    self.browser.select_tab(&mut self.engine, tab);
                }
                if let Some(window) = self.browser.tab(tab).map(|t| t.window) {
                    self.request_redraw(window);
                }
                self.tab_info(&uuid, tab).ok_or_else(missing)
            }
            "tabs.remove" => {
                for tab in tab_ids(&arg(0)) {
                    if self.browser.tab(tab).is_some() {
                        let effects = self.browser.close_tab(&mut self.engine, tab);
                        self.apply(event_loop, effects);
                    }
                }
                Ok(Value::Null)
            }
            "tabs.reload" => {
                let tab = tab_ids(&arg(0)).into_iter().next().ok_or_else(missing)?;
                self.engine.reload(tab, false);
                Ok(Value::Null)
            }
            "windows.get" => {
                let id = arg(0).as_i64().unwrap_or(-2);
                let window = if id < 0 {
                    self.current_window()
                } else {
                    Some(WindowId::from_raw(id as u64))
                        .filter(|w| self.browser.window(*w).is_some())
                }
                .ok_or("Invalid window ID")?;
                Ok(self.window_info(&uuid, window, &arg(1)))
            }
            "windows.getAll" => {
                let windows: Vec<WindowId> = self.browser.windows().iter().map(|w| w.id).collect();
                Ok(Value::Array(
                    windows
                        .into_iter()
                        .map(|w| self.window_info(&uuid, w, &arg(0)))
                        .collect(),
                ))
            }
            "windows.create" => {
                let data = arg(0);
                let url = match data.get("url") {
                    Some(Value::String(url)) => Some(url.clone()),
                    Some(Value::Array(list)) => {
                        list.first().and_then(Value::as_str).map(str::to_owned)
                    }
                    _ => None,
                }
                .and_then(|url| self.extensions.resolve(&uuid, &url));
                let private = data
                    .get("incognito")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let window = self
                    .open_window(event_loop, private, url)
                    .map_err(|e| e.to_string())?;
                Ok(self.window_info(&uuid, window, &json!({ "populate": true })))
            }
            "windows.update" => {
                let window = arg(0)
                    .as_u64()
                    .map(WindowId::from_raw)
                    .ok_or("Invalid window ID")?;
                if arg(1).get("focused").and_then(Value::as_bool) == Some(true)
                    && let Some(w) = self.windows.iter().find(|w| w.id == window)
                {
                    w.window.focus_window();
                }
                Ok(self.window_info(&uuid, window, &json!({})))
            }
            "windows.remove" => {
                let window = arg(0)
                    .as_u64()
                    .map(WindowId::from_raw)
                    .ok_or("Invalid window ID")?;
                let effects = self.browser.close_window(&mut self.engine, window);
                self.apply(event_loop, effects);
                Ok(Value::Null)
            }
            "webNavigation.getFrame" | "webNavigation.getAllFrames" => {
                let details = arg(0);
                let tab = details
                    .get("tabId")
                    .and_then(Value::as_u64)
                    .map(TabId::from_raw)
                    .ok_or("Invalid tab ID")?;
                let t = self.browser.tab(tab).ok_or("Invalid tab ID")?;
                let frame = json!({
                    "tabId": tab.get(),
                    "frameId": 0,
                    "parentFrameId": -1,
                    "url": t.url.as_ref().map(|u| u.as_str()).unwrap_or("about:blank"),
                    "errorOccurred": false,
                });
                if api == "webNavigation.getAllFrames" {
                    Ok(json!([frame]))
                } else if details.get("frameId").and_then(Value::as_i64).unwrap_or(0) == 0 {
                    Ok(frame)
                } else {
                    Ok(Value::Null)
                }
            }
            "tabs.move" => {
                let index = arg(1).get("index").and_then(Value::as_i64).unwrap_or(-1);
                let mut moved = Vec::new();
                for tab in tab_ids(&arg(0)) {
                    let Some(window) = self.browser.tab(tab).map(|t| t.window) else {
                        continue;
                    };
                    let count = self.browser.window(window).map_or(0, |w| w.tabs.len());
                    let target = if index < 0 {
                        count.saturating_sub(1)
                    } else {
                        index as usize
                    };
                    self.browser.move_tab(tab, target);
                    self.request_redraw(window);
                    moved.extend(self.tab_info(&uuid, tab));
                }
                Ok(Value::Array(moved))
            }
            "tabs.duplicate" => {
                let tab = tab_ids(&arg(0)).into_iter().next().ok_or_else(missing)?;
                let (window, url) = self
                    .browser
                    .tab(tab)
                    .and_then(|t| Some((t.window, t.url.clone()?)))
                    .ok_or_else(missing)?;
                let new = self
                    .browser
                    .open_tab(&mut self.engine, window, url, Some(tab), true)
                    .ok_or("Could not open a tab")?;
                self.request_redraw(window);
                Ok(self.tab_info(&uuid, new).unwrap_or(Value::Null))
            }
            "action.getPopup" => Ok(json!(
                self.extensions
                    .popup_url(&uuid)
                    .map(|u| u.to_string())
                    .unwrap_or_default()
            )),
            "runtime.openOptionsPage" => {
                let url = self
                    .extensions
                    .options_url(&uuid)
                    .ok_or("This extension has no options page")?;
                let window = self.current_window().ok_or("No window")?;
                self.browser.open_or_select(&mut self.engine, window, url);
                self.request_redraw(window);
                Ok(Value::Null)
            }
            _ if api.starts_with("action.") => self.action_api(&uuid, api, &arg(0)),
            _ => Err(format!("{api} is not supported yet")),
        }
    }

    fn action_api(&mut self, uuid: &str, api: &str, details: &Value) -> Result<Value, String> {
        let Some(state) = self.extensions.action_mut(uuid) else {
            return Err("This extension has no browser action".into());
        };
        let result = match api {
            "action.setBadgeText" => {
                state.badge_text = details
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(4)
                    .collect();
                Value::Null
            }
            "action.getBadgeText" => json!(state.badge_text),
            "action.setBadgeBackgroundColor" => {
                state.badge_color = match details.get("color") {
                    Some(Value::Array(c)) if c.len() >= 3 => Some([
                        c[0].as_u64().unwrap_or(0) as u8,
                        c[1].as_u64().unwrap_or(0) as u8,
                        c[2].as_u64().unwrap_or(0) as u8,
                        c.get(3).and_then(Value::as_u64).unwrap_or(255) as u8,
                    ]),
                    Some(Value::String(hex)) => parse_color(hex),
                    _ => None,
                };
                Value::Null
            }
            "action.setTitle" => {
                state.title = details
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Value::Null
            }
            "action.getTitle" => json!(state.title.clone().unwrap_or_default()),
            "action.setPopup" => {
                state.popup = Some(
                    details
                        .get("popup")
                        .and_then(Value::as_str)
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned),
                );
                Value::Null
            }
            "action.enable" => {
                state.enabled = true;
                Value::Null
            }
            "action.disable" => {
                state.enabled = false;
                Value::Null
            }
            _ => return Err(format!("{api} is not supported yet")),
        };
        self.refresh_addons();
        Ok(result)
    }

    fn window_info(&self, uuid: &str, window: WindowId, info: &Value) -> Value {
        let Some(w) = self.browser.window(window) else {
            return Value::Null;
        };
        let mut out = json!({
            "id": window.get(),
            "focused": self.current_window() == Some(window),
            "top": 0,
            "left": 0,
            "width": 0,
            "height": 0,
            "incognito": w.private,
            "type": "normal",
            "state": "normal",
            "alwaysOnTop": false,
            "title": "",
        });
        if info.get("populate").and_then(Value::as_bool) == Some(true) {
            out["tabs"] = Value::Array(
                w.tabs
                    .iter()
                    .filter_map(|t| self.tab_info(uuid, *t))
                    .collect(),
            );
        }
        out
    }

    pub(crate) fn extension_event(&mut self, permission: Option<&str>, name: &str, args: Value) {
        let uuids: Vec<String> = match permission {
            Some(permission) => self.extensions.with_permission(permission),
            None => self
                .extensions
                .host
                .extensions()
                .map(|e| e.uuid.clone())
                .collect(),
        };
        for uuid in uuids {
            let out = self.extensions.host.event(
                Some(&uuid),
                Some(klin_extensions::host::Context::Page),
                None,
                name,
                args.clone(),
            );
            for item in out {
                if let Outgoing::Eval { tab, script } = item {
                    self.engine.evaluate(tab, script);
                }
            }
        }
    }

    pub(crate) fn navigation_event(&mut self, name: &str, tab: TabId, url: Option<&Url>) {
        if self.extensions.with_permission("webNavigation").is_empty() {
            return;
        }
        let Some(url) = url
            .or_else(|| self.browser.tab(tab).and_then(|t| t.url.as_ref()))
            .cloned()
        else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https" | "file") {
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        let details = json!({
            "tabId": tab.get(),
            "url": url.as_str(),
            "frameId": 0,
            "parentFrameId": -1,
            "processId": 0,
            "timeStamp": now,
            "transitionType": "link",
            "transitionQualifiers": [],
        });
        self.extension_event(
            Some("webNavigation"),
            &format!("webNavigation.{name}"),
            json!([details]),
        );
    }

    pub(crate) fn notify_tab_updated(&mut self, tab: TabId) {
        if self.browser.tab(tab).is_none() {
            return;
        }
        let uuids: Vec<String> = self
            .extensions
            .host
            .extensions()
            .map(|e| e.uuid.clone())
            .collect();
        for uuid in uuids {
            let Some(info) = self.tab_info(&uuid, tab) else {
                continue;
            };
            let mut change = json!({ "status": info["status"] });
            if let Some(url) = info.get("url") {
                change["url"] = url.clone();
            }
            if let Some(title) = info.get("title") {
                change["title"] = title.clone();
            }
            let out = self.extensions.host.event(
                Some(&uuid),
                Some(klin_extensions::host::Context::Page),
                None,
                "tabs.onUpdated",
                json!([tab.get(), change, info]),
            );
            for item in out {
                if let Outgoing::Eval { tab, script } = item {
                    self.engine.evaluate(tab, script);
                }
            }
        }
    }

    pub(crate) fn persist_extension_uuids(&mut self) {
        let json = self.extensions.uuids_json();
        if self.prefs.string(crate::extensions::UUIDS_PREF, "{}") != json {
            self.prefs.set(
                crate::extensions::UUIDS_PREF,
                klin_core::prefs::PrefValue::String(json),
            );
            self.prefs_changed();
        }
    }

    pub(crate) fn refresh_addons(&mut self) {
        let infos = self.extensions.infos();
        for w in &mut self.windows {
            w.ui.set_addons(infos.clone());
            w.window.request_redraw();
        }
    }

    pub(crate) fn addon_clicked(&mut self, event_loop: &ActiveEventLoop, uuid: &str) {
        if let Some(url) = self.extensions.popup_url(uuid) {
            if let Some(window) = self.current_window() {
                if self.engine.popup_tab(window).is_some() {
                    self.close_popup(window);
                    return;
                }
                self.engine.open_popup(window, TabId::next(), &url);
                self.request_redraw(window);
            }
            return;
        }
        let Some(tab) = self
            .current_window()
            .and_then(|w| self.browser.window(w))
            .and_then(|w| w.selected_tab())
        else {
            return;
        };
        let info = self.tab_info(uuid, tab).unwrap_or(Value::Null);
        let out = self.extensions.host.event(
            Some(uuid),
            Some(klin_extensions::host::Context::Page),
            None,
            "action.onClicked",
            json!([info]),
        );
        self.extension_outgoing(event_loop, out);
    }
}

fn parse_color(hex: &str) -> Option<[u8; 4]> {
    let hex = hex.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        6 => Some([channel(0)?, channel(2)?, channel(4)?, 255]),
        8 => Some([channel(0)?, channel(2)?, channel(4)?, channel(6)?]),
        _ => None,
    }
}
