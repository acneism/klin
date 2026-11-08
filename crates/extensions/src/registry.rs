use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde_json::{Value, json};

use crate::host::{Extension, sanitize};
use crate::manifest::{Manifest, ManifestError};
use crate::package::Package;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub uuid: String,
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("не удалось скопировать дополнение: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Registry {
    profile: PathBuf,
    installed: Vec<Installed>,
    uuids: HashMap<String, String>,
}

pub fn new_uuid() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

impl Registry {
    pub fn load(profile: &Path, uuids: HashMap<String, String>) -> Self {
        let installed = std::fs::read_to_string(profile.join("extensions.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.get("addons").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|addon| {
                let id = addon.get("id")?.as_str()?.to_owned();
                let path = PathBuf::from(addon.get("path")?.as_str()?);
                let enabled = addon.get("active").and_then(Value::as_bool).unwrap_or(true);
                Some((id, path, enabled))
            })
            .collect::<Vec<_>>();
        let mut registry = Self {
            profile: profile.to_owned(),
            installed: Vec::new(),
            uuids,
        };
        for (id, path, enabled) in installed {
            let uuid = registry.uuid_for(&id);
            registry.installed.push(Installed {
                id,
                uuid,
                path,
                enabled,
            });
        }
        registry
    }

    fn uuid_for(&mut self, id: &str) -> String {
        self.uuids
            .entry(id.to_owned())
            .or_insert_with(new_uuid)
            .clone()
    }

    pub fn uuids(&self) -> &HashMap<String, String> {
        &self.uuids
    }

    pub fn installed(&self) -> &[Installed] {
        &self.installed
    }

    fn save(&self) {
        let addons: Vec<Value> = self
            .installed
            .iter()
            .map(|i| {
                json!({
                    "id": i.id,
                    "path": i.path.to_string_lossy(),
                    "active": i.enabled,
                    "location": "app-profile",
                })
            })
            .collect();
        let text = json!({ "schemaVersion": 1, "addons": addons }).to_string();
        let path = self.profile.join("extensions.json");
        let part = path.with_extension("json.tmp");
        if let Err(error) = std::fs::write(&part, text).and_then(|_| std::fs::rename(&part, &path))
        {
            log::error!("{}: {error}", path.display());
        }
    }

    pub fn install(&mut self, source: &Path, ui_locale: &str) -> Result<Installed, InstallError> {
        let package = Package::open(source).map_err(ManifestError::from)?;
        let (manifest, warnings) = Manifest::load(&package, ui_locale)?;
        for warning in warnings {
            log::warn!("{}: {warning}", source.display());
        }
        let id = manifest
            .id
            .clone()
            .unwrap_or_else(|| format!("{{{}}}", new_uuid()));
        let path = if source.is_dir() {
            source.to_owned()
        } else {
            let dir = self.profile.join("extensions");
            std::fs::create_dir_all(&dir)?;
            let target = dir.join(format!("{}.xpi", sanitize(&id)));
            if target != source {
                let part = target.with_extension("xpi.part");
                std::fs::copy(source, &part)?;
                std::fs::rename(&part, &target)?;
            }
            target
        };
        let uuid = self.uuid_for(&id);
        let installed = Installed {
            id: id.clone(),
            uuid,
            path,
            enabled: true,
        };
        self.installed.retain(|i| i.id != id);
        self.installed.push(installed.clone());
        self.save();
        Ok(installed)
    }

    pub fn uninstall(&mut self, id: &str) -> Option<Installed> {
        let index = self.installed.iter().position(|i| i.id == id)?;
        let installed = self.installed.remove(index);
        if installed.path.starts_with(self.profile.join("extensions")) {
            let _ = std::fs::remove_file(&installed.path);
        }
        let _ = std::fs::remove_dir_all(
            self.profile
                .join("browser-extension-data")
                .join(sanitize(id)),
        );
        self.uuids.remove(id);
        self.save();
        Some(installed)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(installed) = self.installed.iter_mut().find(|i| i.id == id) {
            installed.enabled = enabled;
        }
        self.save();
    }
}

impl Installed {
    pub fn open(&self, ui_locale: &str) -> Result<Extension, InstallError> {
        let package = Package::open(&self.path).map_err(ManifestError::from)?;
        let (manifest, _) = Manifest::load(&package, ui_locale)?;
        Ok(Extension {
            id: self.id.clone(),
            uuid: self.uuid.clone(),
            manifest,
            package,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn xpi(dir: &Path, id: Option<&str>) -> PathBuf {
        let path = dir.join("addon.xpi");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        let gecko = id
            .map(|id| format!(r#", "browser_specific_settings": {{"gecko": {{"id": "{id}"}}}}"#))
            .unwrap_or_default();
        write!(
            zip,
            r#"{{"manifest_version": 2, "name": "A", "version": "1"{gecko}}}"#
        )
        .unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn installs_persists_and_uninstalls() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let source = xpi(dir.path(), Some("a@example.com"));
        let mut registry = Registry::load(&profile, HashMap::new());
        let installed = registry.install(&source, "ru").unwrap();
        assert_eq!(installed.path, profile.join("extensions/a@example.com.xpi"));
        assert_eq!(installed.uuid.len(), 36);
        let extension = installed.open("ru").unwrap();
        assert_eq!(extension.manifest.name, "A");
        let uuids = registry.uuids().clone();
        let mut reloaded = Registry::load(&profile, uuids);
        assert_eq!(reloaded.installed(), std::slice::from_ref(&installed));
        reloaded.set_enabled("a@example.com", false);
        assert!(!Registry::load(&profile, reloaded.uuids().clone()).installed()[0].enabled);
        assert!(reloaded.uninstall("a@example.com").is_some());
        assert!(!installed.path.exists());
        assert!(
            Registry::load(&profile, HashMap::new())
                .installed()
                .is_empty()
        );
    }

    #[test]
    fn generates_ids_for_addons_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let source = xpi(dir.path(), None);
        let mut registry = Registry::load(dir.path(), HashMap::new());
        let installed = registry.install(&source, "ru").unwrap();
        assert!(installed.id.starts_with('{') && installed.id.ends_with('}'));
    }
}
