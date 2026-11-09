use std::collections::HashSet;

use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use url::Url;

pub const ALL_TYPES: &[&str] = &[
    "main_frame",
    "sub_frame",
    "stylesheet",
    "script",
    "image",
    "font",
    "object",
    "xmlhttprequest",
    "ping",
    "csp_report",
    "media",
    "websocket",
    "webtransport",
    "webbundle",
    "other",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Allow,
    Block,
    Redirect(String),
}

#[derive(Clone, Debug)]
enum ActionKind {
    Block,
    Allow,
    AllowAllRequests,
    UpgradeScheme,
    Redirect(Redirect),
    Ignored,
}

#[derive(Clone, Debug)]
enum Redirect {
    ExtensionPath(String),
    Url(String),
    RegexSubstitution(String),
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: i64,
    pub priority: i64,
    action: ActionKind,
    pattern: Option<Regex>,
    regex: Option<Regex>,
    types: HashSet<String>,
    methods: Option<HashSet<String>>,
    excluded_methods: HashSet<String>,
    initiator_domains: Option<Vec<String>>,
    excluded_initiator_domains: Vec<String>,
    request_domains: Option<Vec<String>>,
    excluded_request_domains: Vec<String>,
    domain_type: Option<bool>,
    tab_ids: Option<HashSet<i64>>,
    excluded_tab_ids: HashSet<i64>,
    pub raw: Value,
}

#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub url: &'a Url,
    pub initiator: Option<&'a Url>,
    pub kind: &'a str,
    pub method: &'a str,
    pub tab_id: i64,
    pub third_party: bool,
}

fn url_filter_to_regex(filter: &str) -> String {
    let mut rest = filter;
    let mut out = String::new();
    if let Some(r) = rest.strip_prefix("||") {
        out.push_str(r"^[a-z][a-z0-9+.-]*:(?://)?(?:[^/?#]*@)?(?:[^/?#.]+\.)*");
        rest = r;
    } else if let Some(r) = rest.strip_prefix('|') {
        out.push('^');
        rest = r;
    }
    let end_anchor = rest.ends_with('|') && !rest.ends_with("\\|");
    if end_anchor {
        rest = &rest[..rest.len() - 1];
    }
    for c in rest.chars() {
        match c {
            '*' => out.push_str(".*"),
            '^' => out.push_str(r"(?:[^a-zA-Z0-9_.%-]|$)"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
    }
    if end_anchor {
        out.push('$');
    }
    out
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(Value::as_str)
            .map(|s| s.to_ascii_lowercase())
            .collect()
    })
}

fn domain_matches(host: &str, domains: &[String]) -> bool {
    domains
        .iter()
        .any(|d| host == d || host.ends_with(&format!(".{d}")))
}

impl Rule {
    pub fn parse(raw: &Value) -> Result<Self, String> {
        let id = raw
            .get("id")
            .and_then(Value::as_i64)
            .filter(|id| *id >= 1)
            .ok_or("rule id must be a positive integer")?;
        let priority = raw.get("priority").and_then(Value::as_i64).unwrap_or(1);
        if priority < 1 {
            return Err(format!("rule {id}: priority must be at least 1"));
        }
        let action = raw.get("action").ok_or(format!("rule {id}: no action"))?;
        let condition = raw.get("condition").cloned().unwrap_or(json!({}));
        let action = match action.get("type").and_then(Value::as_str) {
            Some("block") => ActionKind::Block,
            Some("allow") => ActionKind::Allow,
            Some("allowAllRequests") => ActionKind::AllowAllRequests,
            Some("upgradeScheme") => ActionKind::UpgradeScheme,
            Some("modifyHeaders") => ActionKind::Ignored,
            Some("redirect") => {
                let redirect = action
                    .get("redirect")
                    .ok_or(format!("rule {id}: no redirect"))?;
                if let Some(path) = redirect.get("extensionPath").and_then(Value::as_str) {
                    ActionKind::Redirect(Redirect::ExtensionPath(path.to_owned()))
                } else if let Some(url) = redirect.get("url").and_then(Value::as_str) {
                    ActionKind::Redirect(Redirect::Url(url.to_owned()))
                } else if let Some(sub) = redirect.get("regexSubstitution").and_then(Value::as_str)
                {
                    ActionKind::Redirect(Redirect::RegexSubstitution(sub.to_owned()))
                } else {
                    ActionKind::Ignored
                }
            }
            other => return Err(format!("rule {id}: unknown action {other:?}")),
        };
        let case_sensitive = condition
            .get("isUrlFilterCaseSensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let build = |source: &str| {
            RegexBuilder::new(source)
                .case_insensitive(!case_sensitive)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| format!("rule {id}: {e}"))
        };
        let pattern = condition
            .get("urlFilter")
            .and_then(Value::as_str)
            .map(|f| build(&url_filter_to_regex(f)))
            .transpose()?;
        let regex = condition
            .get("regexFilter")
            .and_then(Value::as_str)
            .map(build)
            .transpose()?;
        let mut types: HashSet<String> = match strings(condition.get("resourceTypes")) {
            Some(list) => list.into_iter().collect(),
            None => ALL_TYPES
                .iter()
                .filter(|t| **t != "main_frame")
                .map(|t| (*t).to_owned())
                .collect(),
        };
        for excluded in strings(condition.get("excludedResourceTypes")).unwrap_or_default() {
            types.remove(&excluded);
        }
        let ids = |key: &str| {
            condition.get(key).and_then(Value::as_array).map(|list| {
                list.iter()
                    .filter_map(Value::as_i64)
                    .collect::<HashSet<i64>>()
            })
        };
        let domains = |key: &str, legacy: &str| {
            strings(condition.get(key)).or_else(|| strings(condition.get(legacy)))
        };
        Ok(Rule {
            id,
            priority,
            action,
            pattern,
            regex,
            types,
            methods: strings(condition.get("requestMethods")).map(|l| l.into_iter().collect()),
            excluded_methods: strings(condition.get("excludedRequestMethods"))
                .unwrap_or_default()
                .into_iter()
                .collect(),
            initiator_domains: domains("initiatorDomains", "domains"),
            excluded_initiator_domains: domains("excludedInitiatorDomains", "excludedDomains")
                .unwrap_or_default(),
            request_domains: strings(condition.get("requestDomains")),
            excluded_request_domains: strings(condition.get("excludedRequestDomains"))
                .unwrap_or_default(),
            domain_type: match condition.get("domainType").and_then(Value::as_str) {
                Some("thirdParty") => Some(true),
                Some("firstParty") => Some(false),
                _ => None,
            },
            tab_ids: ids("tabIds"),
            excluded_tab_ids: ids("excludedTabIds").unwrap_or_default(),
            raw: raw.clone(),
        })
    }

    fn matches(&self, request: &Request<'_>) -> bool {
        if !self.types.contains(request.kind) {
            return false;
        }
        let method = request.method.to_ascii_lowercase();
        if self.methods.as_ref().is_some_and(|m| !m.contains(&method))
            || self.excluded_methods.contains(&method)
        {
            return false;
        }
        if self
            .domain_type
            .is_some_and(|third| third != request.third_party)
        {
            return false;
        }
        if self
            .tab_ids
            .as_ref()
            .is_some_and(|t| !t.contains(&request.tab_id))
            || self.excluded_tab_ids.contains(&request.tab_id)
        {
            return false;
        }
        let host = request
            .url
            .host_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if self
            .request_domains
            .as_ref()
            .is_some_and(|d| !domain_matches(&host, d))
            || domain_matches(&host, &self.excluded_request_domains)
        {
            return false;
        }
        let initiator = request
            .initiator
            .and_then(|u| u.host_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if self
            .initiator_domains
            .as_ref()
            .is_some_and(|d| !domain_matches(&initiator, d))
            || domain_matches(&initiator, &self.excluded_initiator_domains)
        {
            return false;
        }
        let url = request.url.as_str();
        if self.pattern.as_ref().is_some_and(|p| !p.is_match(url)) {
            return false;
        }
        if self.regex.as_ref().is_some_and(|r| !r.is_match(url)) {
            return false;
        }
        true
    }

    fn rank(&self) -> u8 {
        match self.action {
            ActionKind::Allow => 5,
            ActionKind::AllowAllRequests => 4,
            ActionKind::Block => 3,
            ActionKind::UpgradeScheme => 2,
            ActionKind::Redirect(_) => 1,
            ActionKind::Ignored => 0,
        }
    }

    fn outcome(&self, request: &Request<'_>, uuid: &str) -> Option<Outcome> {
        match &self.action {
            ActionKind::Allow | ActionKind::AllowAllRequests => Some(Outcome::Allow),
            ActionKind::Block => Some(Outcome::Block),
            ActionKind::UpgradeScheme => {
                if request.url.scheme() != "http" {
                    return Some(Outcome::Allow);
                }
                let mut url = request.url.clone();
                url.set_scheme("https").ok()?;
                Some(Outcome::Redirect(url.into()))
            }
            ActionKind::Redirect(Redirect::ExtensionPath(path)) => Some(Outcome::Redirect(
                format!("moz-extension://{uuid}/{}", path.trim_start_matches('/')),
            )),
            ActionKind::Redirect(Redirect::Url(url)) => Some(Outcome::Redirect(url.clone())),
            ActionKind::Redirect(Redirect::RegexSubstitution(sub)) => {
                let regex = self.regex.as_ref()?;
                let mut template = String::new();
                let mut chars = sub.chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        '$' => template.push_str("$$"),
                        '\\' if chars.peek().is_some_and(char::is_ascii_digit) => {
                            let digit = chars.next().unwrap_or('0');
                            template.push_str(&format!("${{{digit}}}"));
                        }
                        c => template.push(c),
                    }
                }
                let replaced = regex.replace(request.url.as_str(), template.as_str());
                Some(Outcome::Redirect(replaced.into_owned()))
            }
            ActionKind::Ignored => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn parse(rules: &Value) -> Result<Self, String> {
        let list = rules.as_array().ok_or("rules must be an array")?;
        let mut seen = HashSet::new();
        let mut out = Vec::with_capacity(list.len());
        for raw in list {
            let rule = Rule::parse(raw)?;
            if !seen.insert(rule.id) {
                return Err(format!("duplicate rule id {}", rule.id));
            }
            out.push(rule);
        }
        Ok(Self { rules: out })
    }

    pub fn best<'a>(&'a self, request: &Request<'_>) -> Option<&'a Rule> {
        self.rules
            .iter()
            .filter(|r| !matches!(r.action, ActionKind::Ignored) && r.matches(request))
            .max_by_key(|r| (r.priority, r.rank()))
    }

    pub fn remove(&mut self, ids: &HashSet<i64>) {
        self.rules.retain(|r| !ids.contains(&r.id));
    }

    pub fn to_json(&self) -> Value {
        Value::Array(self.rules.iter().map(|r| r.raw.clone()).collect())
    }
}

pub fn evaluate<'a>(
    sets: impl Iterator<Item = &'a RuleSet>,
    request: &Request<'_>,
    uuid: &str,
) -> Option<Outcome> {
    let best = sets
        .filter_map(|set| set.best(request))
        .max_by_key(|r| (r.priority, r.rank()))?;
    best.outcome(request, uuid)
}

pub fn regex_supported(source: &str) -> bool {
    RegexBuilder::new(source)
        .size_limit(1 << 20)
        .build()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn request<'a>(url: &'a Url, initiator: Option<&'a Url>, kind: &'a str) -> Request<'a> {
        Request {
            url,
            initiator,
            kind,
            method: "GET",
            tab_id: 1,
            third_party: true,
        }
    }

    fn set(rules: Value) -> RuleSet {
        RuleSet::parse(&rules).unwrap()
    }

    #[test]
    fn url_filters_follow_the_dnr_syntax() {
        let rules = set(json!([
            {"id": 1, "action": {"type": "block"}, "condition": {"urlFilter": "||ads.example^"}},
            {"id": 2, "action": {"type": "block"}, "condition": {"urlFilter": "|https://track.*/pixel|"}},
            {"id": 3, "action": {"type": "block"}, "condition": {"urlFilter": "banner", "resourceTypes": ["image"]}}
        ]));
        let check = |url: &str, kind: &str| {
            let url = u(url);
            rules.best(&request(&url, None, kind)).map(|r| r.id)
        };
        assert_eq!(check("https://ads.example/x.js", "script"), Some(1));
        assert_eq!(check("https://cdn.ads.example:8080/x", "script"), Some(1));
        assert_eq!(check("https://notads.example/x", "script"), None);
        assert_eq!(check("https://ads.example.com/x", "script"), None);
        assert_eq!(check("https://track.example.org/pixel", "image"), Some(2));
        assert_eq!(check("https://track.example.org/pixel?x", "image"), None);
        assert_eq!(check("https://a.example/BANNER.png", "image"), Some(3));
        assert_eq!(check("https://a.example/banner.js", "script"), None);
        assert_eq!(check("https://ads.example/", "main_frame"), None);
    }

    #[test]
    fn priorities_and_allow_rules_decide() {
        let rules = set(json!([
            {"id": 1, "action": {"type": "block"}, "condition": {"requestDomains": ["cdn.example"]}},
            {"id": 2, "priority": 2, "action": {"type": "allow"}, "condition": {"urlFilter": "/ok/", "initiatorDomains": ["good.example"]}},
            {"id": 3, "action": {"type": "redirect", "redirect": {"extensionPath": "/noop.js"}}, "condition": {"urlFilter": "lib.js", "resourceTypes": ["script"]}},
            {"id": 4, "action": {"type": "upgradeScheme"}, "condition": {"requestDomains": ["plain.example"], "resourceTypes": ["main_frame"]}}
        ]));
        let good = u("https://www.good.example/");
        let blocked = u("https://cdn.example/ok/a.js");
        assert_eq!(
            evaluate(
                [&rules].into_iter(),
                &request(&blocked, None, "script"),
                "uuid"
            ),
            Some(Outcome::Block)
        );
        assert_eq!(
            evaluate(
                [&rules].into_iter(),
                &request(&blocked, Some(&good), "script"),
                "uuid"
            ),
            Some(Outcome::Allow)
        );
        let lib = u("https://x.example/lib.js");
        assert_eq!(
            evaluate([&rules].into_iter(), &request(&lib, None, "script"), "uuid"),
            Some(Outcome::Redirect("moz-extension://uuid/noop.js".into()))
        );
        let plain = u("http://plain.example/page");
        assert_eq!(
            evaluate(
                [&rules].into_iter(),
                &request(&plain, None, "main_frame"),
                "uuid"
            ),
            Some(Outcome::Redirect("https://plain.example/page".into()))
        );
    }

    #[test]
    fn regex_substitution_and_validation() {
        let rules = set(json!([
            {"id": 1, "action": {"type": "redirect", "redirect": {"regexSubstitution": "https://mirror.example/\\1"}},
             "condition": {"regexFilter": "^https://origin\\.example/(.*)$"}}
        ]));
        let url = u("https://origin.example/a/b.js");
        assert_eq!(
            evaluate([&rules].into_iter(), &request(&url, None, "script"), "u"),
            Some(Outcome::Redirect("https://mirror.example/a/b.js".into()))
        );
        assert!(RuleSet::parse(&json!([{"id": 1, "action": {"type": "block"}}, {"id": 1, "action": {"type": "block"}}])).is_err());
        assert!(RuleSet::parse(&json!([{"id": 0, "action": {"type": "block"}}])).is_err());
        assert!(RuleSet::parse(&json!([{"id": 1, "action": {"type": "explode"}}])).is_err());
        assert!(regex_supported("^a+$"));
        assert!(!regex_supported("("));
    }
}
