use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use klin_engine::ServoEngine;
use klin_extensions::host::{BACKGROUND_PAGE, Extension, Host, Outgoing};
use klin_extensions::registry::Registry;
use klin_extensions::resources::ExtensionFiles;
use klin_extensions::script::{ScriptConfig, background_page, user_script};
use klin_shared::addons::{AddonAction, AddonInfo};
use klin_shared::{Image, TabId, Url};

pub const UUIDS_PREF: &str = "extensions.webextensions.uuids";
pub const UI_LOCALE: &str = "ru";

#[derive(Clone, Debug, Default)]
pub struct ActionState {
    pub title: Option<String>,
    pub badge_text: String,
    pub badge_color: Option<[u8; 4]>,
    pub popup: Option<Option<String>>,
    pub enabled: bool,
}

pub struct Extensions {
    registry: Registry,
    pub host: Host,
    files: ExtensionFiles,
    backgrounds: HashMap<String, TabId>,
    actions: HashMap<String, ActionState>,
    icons: HashMap<String, Option<Arc<Image>>>,
    warnings: HashMap<String, Vec<String>>,
    started: bool,
}

pub fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

pub fn parse_uuids(text: &str) -> HashMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

impl Extensions {
    pub fn new(profile: &Path, uuids: &str, engine: &ServoEngine) -> Self {
        Self {
            registry: Registry::load(profile, parse_uuids(uuids)),
            host: Host::new(profile.to_owned()),
            files: engine.extension_files(),
            backgrounds: HashMap::new(),
            actions: HashMap::new(),
            icons: HashMap::new(),
            warnings: HashMap::new(),
            started: false,
        }
    }

    pub fn uuids_json(&self) -> String {
        serde_json::to_string(self.registry.uuids()).unwrap_or_else(|_| "{}".into())
    }

    pub fn load(&mut self, engine: &mut ServoEngine) {
        let installed = self.registry.installed().to_vec();
        for item in installed.into_iter().filter(|i| i.enabled) {
            match item.open(UI_LOCALE) {
                Ok(extension) => self.activate(extension),
                Err(error) => {
                    log::error!("{}: {error}", item.id);
                    self.warnings
                        .insert(item.id.clone(), vec![error.to_string()]);
                }
            }
        }
        self.update_scripts(engine);
    }

    fn activate(&mut self, extension: Extension) {
        let background = background_page(&extension.manifest);
        self.files
            .insert(&extension.uuid, extension.package.clone(), background);
        let icon = extension
            .manifest
            .best_icon(32)
            .and_then(|path| extension.package.read(path).ok())
            .and_then(|bytes| decode_png(&bytes))
            .map(Arc::new);
        self.icons.insert(extension.uuid.clone(), icon);
        self.actions.insert(
            extension.uuid.clone(),
            ActionState {
                enabled: true,
                ..Default::default()
            },
        );
        self.host.add(extension);
    }

    fn update_scripts(&self, engine: &mut ServoEngine) {
        let version = env!("CARGO_PKG_VERSION");
        let scripts = self
            .host
            .extensions()
            .map(|e| {
                user_script(&ScriptConfig {
                    uuid: &e.uuid,
                    id: &e.id,
                    manifest: &e.manifest,
                    package: &e.package,
                    ui_language: UI_LOCALE,
                    browser_version: version,
                })
            })
            .collect();
        engine.set_extension_scripts(scripts);
    }

    pub fn start_backgrounds(&mut self, engine: &mut ServoEngine) -> Vec<Outgoing> {
        if self.started {
            return Vec::new();
        }
        self.started = true;
        let uuids: Vec<String> = self.host.extensions().map(|e| e.uuid.clone()).collect();
        for uuid in uuids {
            self.start_background(&uuid, engine);
        }
        Vec::new()
    }

    fn start_background(&mut self, uuid: &str, engine: &mut ServoEngine) {
        let Some(extension) = self.host.extension(uuid) else {
            return;
        };
        let page = match &extension.manifest.background {
            Some(klin_extensions::manifest::Background::Scripts(_)) => BACKGROUND_PAGE.to_owned(),
            Some(klin_extensions::manifest::Background::Page(page)) => {
                page.trim_start_matches('/').to_owned()
            }
            None => return,
        };
        let Ok(url) = Url::parse(&format!("moz-extension://{uuid}/{page}")) else {
            return;
        };
        let tab = TabId::next();
        if engine.open_extension_view(tab, &url) {
            self.backgrounds.insert(uuid.to_owned(), tab);
        }
    }

    pub fn install(&mut self, path: &Path, engine: &mut ServoEngine) -> Result<String, String> {
        let installed = self
            .registry
            .install(path, UI_LOCALE)
            .map_err(|e| e.to_string())?;
        if let Some(old) = self.host.remove(&installed.uuid) {
            self.deactivate(&old.uuid, engine);
        }
        let extension = installed.open(UI_LOCALE).map_err(|e| e.to_string())?;
        let name = extension.manifest.name.clone();
        let uuid = extension.uuid.clone();
        self.activate(extension);
        self.update_scripts(engine);
        if self.started {
            self.start_background(&uuid, engine);
        }
        Ok(name)
    }

    fn deactivate(&mut self, uuid: &str, engine: &mut ServoEngine) {
        self.files.remove(uuid);
        self.actions.remove(uuid);
        if let Some(tab) = self.backgrounds.remove(uuid) {
            klin_core::Engine::close_tab(engine, tab);
        }
    }

    pub fn uninstall(&mut self, id: &str, engine: &mut ServoEngine) {
        if let Some(installed) = self.registry.uninstall(id) {
            self.host.remove(&installed.uuid);
            self.deactivate(&installed.uuid, engine);
            self.update_scripts(engine);
        }
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool, engine: &mut ServoEngine) {
        self.registry.set_enabled(id, enabled);
        let Some(installed) = self
            .registry
            .installed()
            .iter()
            .find(|i| i.id == id)
            .cloned()
        else {
            return;
        };
        if enabled {
            if self.host.extension(&installed.uuid).is_none()
                && let Ok(extension) = installed.open(UI_LOCALE)
            {
                self.activate(extension);
                if self.started {
                    self.start_background(&installed.uuid, engine);
                }
            }
        } else if self.host.remove(&installed.uuid).is_some() {
            self.deactivate(&installed.uuid, engine);
        }
        self.update_scripts(engine);
    }

    pub fn infos(&self) -> Vec<AddonInfo> {
        let mut infos: Vec<AddonInfo> = self
            .registry
            .installed()
            .iter()
            .map(|installed| {
                let active = self.host.extension(&installed.uuid);
                let manifest = active.map(|e| &e.manifest);
                let action = manifest.and_then(|m| m.action.as_ref()).map(|action| {
                    let state = self
                        .actions
                        .get(&installed.uuid)
                        .cloned()
                        .unwrap_or_default();
                    AddonAction {
                        title: state
                            .title
                            .or_else(|| action.title.clone())
                            .or_else(|| manifest.map(|m| m.name.clone()))
                            .unwrap_or_default(),
                        badge_text: state.badge_text,
                        badge_color: state.badge_color.unwrap_or([0xd9, 0x00, 0x00, 0xff]),
                        has_popup: state
                            .popup
                            .clone()
                            .unwrap_or_else(|| action.popup.clone())
                            .is_some(),
                        enabled: state.enabled,
                        icon: self.icons.get(&installed.uuid).cloned().flatten(),
                    }
                });
                AddonInfo {
                    id: installed.id.clone(),
                    uuid: installed.uuid.clone(),
                    name: manifest
                        .map(|m| m.name.clone())
                        .unwrap_or_else(|| installed.id.clone()),
                    version: manifest.map(|m| m.version.clone()).unwrap_or_default(),
                    description: manifest.map(|m| m.description.clone()).unwrap_or_default(),
                    enabled: installed.enabled,
                    has_options: manifest.and_then(|m| m.options_page.as_ref()).is_some(),
                    warnings: self
                        .warnings
                        .get(&installed.id)
                        .cloned()
                        .unwrap_or_default(),
                    action,
                }
            })
            .collect();
        infos.sort_by_key(|i| i.name.to_lowercase());
        infos
    }

    pub fn popup_url(&self, uuid: &str) -> Option<Url> {
        let extension = self.host.extension(uuid)?;
        let state = self.actions.get(uuid)?;
        let popup = state.popup.clone().unwrap_or_else(|| {
            extension
                .manifest
                .action
                .as_ref()
                .and_then(|a| a.popup.clone())
        })?;
        Url::parse(&format!(
            "moz-extension://{uuid}/{}",
            popup.trim_start_matches('/')
        ))
        .ok()
    }

    pub fn options_url(&self, uuid: &str) -> Option<Url> {
        let page = self.host.extension(uuid)?.manifest.options_page.clone()?;
        Url::parse(&format!(
            "moz-extension://{uuid}/{}",
            page.trim_start_matches('/')
        ))
        .ok()
    }

    pub fn resolve(&self, uuid: &str, url: &str) -> Option<Url> {
        Url::parse(url).ok().or_else(|| {
            Url::parse(&format!("moz-extension://{uuid}/"))
                .ok()?
                .join(url)
                .ok()
        })
    }

    pub fn action_mut(&mut self, uuid: &str) -> Option<&mut ActionState> {
        self.actions.get_mut(uuid)
    }
}
