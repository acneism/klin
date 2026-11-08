use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use crate::Lists;

pub const BLOCKLIST_URL: &str = "https://raw.githubusercontent.com/mozilla-services/shavar-prod-lists/master/disconnect-blacklist.json";
pub const ENTITYLIST_URL: &str = "https://raw.githubusercontent.com/mozilla-services/shavar-prod-lists/master/disconnect-entitylist.json";

const BLOCKLIST_FILE: &str = "disconnect-blacklist.json";
const ENTITYLIST_FILE: &str = "disconnect-entitylist.json";
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const MAX_LIST_SIZE: u64 = 32 * 1024 * 1024;

pub struct ListUpdater {
    dir: PathBuf,
    user_agent: String,
}

fn load(dir: &Path) -> Option<Lists> {
    let blocklist = std::fs::read_to_string(dir.join(BLOCKLIST_FILE)).ok()?;
    let entities = std::fs::read_to_string(dir.join(ENTITYLIST_FILE)).ok();
    match Lists::parse(&blocklist, entities.as_deref()) {
        Ok(lists) => Some(lists),
        Err(error) => {
            log::warn!("tracking protection lists in {}: {error}", dir.display());
            None
        }
    }
}

fn age(dir: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(dir.join(BLOCKLIST_FILE))
        .and_then(|m| m.modified())
        .ok()?;
    SystemTime::now().duration_since(modified).ok()
}

impl ListUpdater {
    pub fn new(dir: PathBuf, user_agent: String) -> Self {
        Self { dir, user_agent }
    }

    pub fn cached(&self) -> Option<Lists> {
        load(&self.dir)
    }

    fn fetch(&self, url: &str) -> Result<String, String> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(true)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent();
        let response = agent
            .get(url)
            .header("User-Agent", &self.user_agent)
            .call()
            .map_err(|e| e.to_string())?;
        let mut body = String::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_LIST_SIZE)
            .read_to_string(&mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    }

    fn write(&self, name: &str, contents: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let part = self.dir.join(format!("{name}.part"));
        std::fs::write(&part, contents)?;
        std::fs::rename(part, self.dir.join(name))
    }

    pub fn update(&self) -> Result<Lists, String> {
        let blocklist = self.fetch(BLOCKLIST_URL)?;
        let entities = self.fetch(ENTITYLIST_URL)?;
        let lists = Lists::parse(&blocklist, Some(&entities)).map_err(|e| e.to_string())?;
        self.write(ENTITYLIST_FILE, &entities)
            .and_then(|_| self.write(BLOCKLIST_FILE, &blocklist))
            .map_err(|e| e.to_string())?;
        Ok(lists)
    }

    pub fn spawn(self, sender: Sender<Arc<Lists>>, wake: impl Fn() + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name("tracking-lists".into())
            .spawn(move || {
                let send = |lists: Lists| {
                    let ok = sender.send(Arc::new(lists)).is_ok();
                    wake();
                    ok
                };
                if let Some(lists) = self.cached()
                    && !send(lists)
                {
                    return;
                }
                loop {
                    let due = age(&self.dir)
                        .map_or(Duration::ZERO, |age| UPDATE_INTERVAL.saturating_sub(age));
                    std::thread::sleep(due);
                    match self.update() {
                        Ok(lists) => {
                            log::info!("tracking protection lists updated: {} hosts", lists.len());
                            if !send(lists) {
                                return;
                            }
                        }
                        Err(error) => {
                            log::warn!("tracking protection lists update failed: {error}");
                            std::thread::sleep(RETRY_INTERVAL);
                        }
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("tracking protection lists thread: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_cached_lists_from_profile() {
        let dir = std::env::temp_dir().join(format!("klin-cb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let updater = ListUpdater::new(dir.clone(), "test".into());
        assert!(updater.cached().is_none());
        updater
            .write(
                BLOCKLIST_FILE,
                r#"{"categories": {"Advertising": [{"A": {"https://a/": ["a.example"]}}]}}"#,
            )
            .unwrap();
        assert_eq!(updater.cached().map(|l| l.len()), Some(1));
        assert!(age(&dir).is_some_and(|a| a < UPDATE_INTERVAL));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "network"]
    fn downloads_current_lists() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("klin-cb-net-{}", std::process::id()));
        let updater = ListUpdater::new(dir.clone(), "Klin test".into());
        let lists = updater.update().unwrap();
        assert!(lists.len() > 1000);
        assert!(updater.cached().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
