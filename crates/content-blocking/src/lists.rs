use std::collections::{HashMap, HashSet};

use serde_json::Value;

pub(crate) const ADVERTISING: u16 = 1;
pub(crate) const ANALYTICS: u16 = 1 << 1;
pub(crate) const SOCIAL: u16 = 1 << 2;
pub(crate) const CONTENT: u16 = 1 << 3;
pub(crate) const CRYPTOMINING: u16 = 1 << 4;
pub(crate) const FINGERPRINTING: u16 = 1 << 5;
pub(crate) const EMAIL: u16 = 1 << 6;
pub(crate) const TRACKING: u16 = ADVERTISING | ANALYTICS | SOCIAL;

const MAX_HOST_COMPONENTS: usize = 5;

#[derive(Debug)]
pub struct ListsError(String);

impl std::fmt::Display for ListsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ListsError {}

#[derive(Debug, Default)]
pub struct Lists {
    hosts: HashMap<String, u16>,
    entities: Vec<HashSet<String>>,
    properties: HashMap<String, Vec<usize>>,
}

fn category_flag(name: &str) -> u16 {
    match name {
        "Advertising" => ADVERTISING,
        "Analytics" => ANALYTICS,
        "Social" | "Disconnect" => SOCIAL,
        "Content" => CONTENT,
        "Cryptomining" => CRYPTOMINING,
        "FingerprintingInvasive" => FINGERPRINTING,
        "Email" => EMAIL,
        _ => 0,
    }
}

fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    let host = host.trim_end_matches('.');
    let labels = host.split('.').count();
    let skip = labels.saturating_sub(MAX_HOST_COMPONENTS);
    let mut rest = Some(host);
    for _ in 0..skip {
        rest = rest.and_then(|h| h.split_once('.').map(|(_, r)| r));
    }
    std::iter::from_fn(move || {
        let current = rest?;
        rest = current
            .split_once('.')
            .map(|(_, r)| r)
            .filter(|r| r.contains('.'));
        Some(current)
    })
}

impl Lists {
    pub fn parse(blocklist: &str, entitylist: Option<&str>) -> Result<Self, ListsError> {
        let value: Value =
            serde_json::from_str(blocklist).map_err(|e| ListsError(format!("blocklist: {e}")))?;
        let categories = value
            .get("categories")
            .and_then(Value::as_object)
            .ok_or_else(|| ListsError("blocklist: no categories".into()))?;
        let mut lists = Lists::default();
        for (name, entries) in categories {
            let flag = category_flag(name);
            if flag == 0 {
                continue;
            }
            for entry in entries.as_array().into_iter().flatten() {
                for organization in entry.as_object().into_iter().flat_map(|o| o.values()) {
                    for domains in organization
                        .as_object()
                        .into_iter()
                        .flat_map(|o| o.values())
                    {
                        for domain in domains.as_array().into_iter().flatten() {
                            if let Some(domain) = domain.as_str() {
                                *lists.hosts.entry(domain.to_ascii_lowercase()).or_default() |=
                                    flag;
                            }
                        }
                    }
                }
            }
        }
        if let Some(entitylist) = entitylist {
            lists.parse_entities(entitylist)?;
        }
        Ok(lists)
    }

    fn parse_entities(&mut self, entitylist: &str) -> Result<(), ListsError> {
        let value: Value =
            serde_json::from_str(entitylist).map_err(|e| ListsError(format!("entitylist: {e}")))?;
        let entities = value
            .get("entities")
            .and_then(Value::as_object)
            .ok_or_else(|| ListsError("entitylist: no entities".into()))?;
        let strings = |entity: &Value, key: &str| -> Vec<String> {
            entity
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_ascii_lowercase)
                .collect()
        };
        for entity in entities.values() {
            let index = self.entities.len();
            self.entities
                .push(strings(entity, "resources").into_iter().collect());
            for property in strings(entity, "properties") {
                self.properties.entry(property).or_default().push(index);
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.hosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    pub(crate) fn flags(&self, host: &str) -> u16 {
        suffixes(host)
            .filter_map(|h| self.hosts.get(h))
            .fold(0, |acc, f| acc | f)
    }

    pub(crate) fn same_entity(&self, top_host: &str, host: &str) -> bool {
        suffixes(top_host)
            .filter_map(|h| self.properties.get(h))
            .flatten()
            .any(|&entity| suffixes(host).any(|h| self.entities[entity].contains(h)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_suffixes_stop_before_tld() {
        let all: Vec<&str> = suffixes("a.b.example.com").collect();
        assert_eq!(all, ["a.b.example.com", "b.example.com", "example.com"]);
        let long: Vec<&str> = suffixes("1.2.3.4.5.6.example.com").collect();
        assert_eq!(long[0], "4.5.6.example.com");
        assert_eq!(suffixes("localhost").collect::<Vec<_>>(), ["localhost"]);
    }

    #[test]
    fn rejects_malformed_lists() {
        assert!(Lists::parse("{}", None).is_err());
        assert!(Lists::parse("not json", None).is_err());
        let lists = Lists::parse(r#"{"categories": {"Advertising": []}}"#, Some("{}"));
        assert!(lists.is_err());
    }
}
