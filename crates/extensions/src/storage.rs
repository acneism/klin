use std::path::PathBuf;

use serde_json::{Map, Value, json};

pub struct StorageArea {
    path: Option<PathBuf>,
    data: Map<String, Value>,
}

impl StorageArea {
    pub fn open(path: Option<PathBuf>) -> Self {
        let data = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default();
        Self { path, data }
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let text = Value::Object(self.data.clone()).to_string();
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| {
                let part = path.with_extension("js.tmp");
                std::fs::write(&part, text)?;
                std::fs::rename(&part, path)
            });
        if let Err(error) = result {
            log::error!("extension storage {}: {error}", path.display());
        }
    }

    pub fn get(&self, keys: &Value) -> Value {
        match keys {
            Value::Null => Value::Object(self.data.clone()),
            Value::String(key) => {
                let mut out = Map::new();
                if let Some(value) = self.data.get(key) {
                    out.insert(key.clone(), value.clone());
                }
                Value::Object(out)
            }
            Value::Array(list) => Value::Object(
                list.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|k| self.data.get(k).map(|v| (k.to_owned(), v.clone())))
                    .collect(),
            ),
            Value::Object(defaults) => Value::Object(
                defaults
                    .iter()
                    .map(|(k, d)| (k.clone(), self.data.get(k).unwrap_or(d).clone()))
                    .collect(),
            ),
            _ => json!({}),
        }
    }

    pub fn set(&mut self, items: &Value) -> Map<String, Value> {
        let mut changes = Map::new();
        if let Value::Object(items) = items {
            for (key, value) in items {
                let mut change = Map::new();
                if let Some(old) = self.data.get(key) {
                    change.insert("oldValue".into(), old.clone());
                }
                change.insert("newValue".into(), value.clone());
                self.data.insert(key.clone(), value.clone());
                changes.insert(key.clone(), Value::Object(change));
            }
            self.save();
        }
        changes
    }

    pub fn remove(&mut self, keys: &Value) -> Map<String, Value> {
        let keys: Vec<String> = match keys {
            Value::String(key) => vec![key.clone()],
            Value::Array(list) => list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let mut changes = Map::new();
        for key in keys {
            if let Some(old) = self.data.remove(&key) {
                changes.insert(key, json!({ "oldValue": old }));
            }
        }
        if !changes.is_empty() {
            self.save();
        }
        changes
    }

    pub fn clear(&mut self) -> Map<String, Value> {
        let changes: Map<String, Value> = std::mem::take(&mut self.data)
            .into_iter()
            .map(|(k, v)| (k, json!({ "oldValue": v })))
            .collect();
        self.save();
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_like_storage_local() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ext").join("storage.js");
        let mut area = StorageArea::open(Some(path.clone()));
        let changes = area.set(&json!({"a": 1, "b": {"c": [1, 2]}}));
        assert_eq!(changes["a"], json!({"newValue": 1}));
        assert_eq!(
            area.set(&json!({"a": 2}))["a"],
            json!({"oldValue": 1, "newValue": 2})
        );
        assert_eq!(area.get(&json!("a")), json!({"a": 2}));
        assert_eq!(area.get(&json!(["a", "zzz"])), json!({"a": 2}));
        assert_eq!(
            area.get(&json!({"a": 0, "d": "x"})),
            json!({"a": 2, "d": "x"})
        );
        let reopened = StorageArea::open(Some(path));
        assert_eq!(
            reopened.get(&Value::Null),
            json!({"a": 2, "b": {"c": [1, 2]}})
        );
        assert_eq!(area.remove(&json!("a"))["a"], json!({"oldValue": 2}));
        assert_eq!(area.clear().len(), 1);
        assert_eq!(area.get(&Value::Null), json!({}));
    }
}
