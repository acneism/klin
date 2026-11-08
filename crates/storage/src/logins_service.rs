use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use klin_shared::logins::{ImportedLogin, LoginInput, SavedLogin};

use crate::logins::{
    Login, LoginEntry, LoginEntryWithMeta, LoginMeta, LoginStore, ManagedEncryptorDecryptor,
};
use crate::support::crypto::{CryptoError, CryptoResult, KeyManager, create_key};

pub type Protect = fn(&[u8]) -> io::Result<Vec<u8>>;

pub struct ProtectedKeyFile {
    path: PathBuf,
    protect: Protect,
    unprotect: Protect,
    cached: Mutex<Option<Vec<u8>>>,
}

impl ProtectedKeyFile {
    pub fn new(path: PathBuf, protect: Protect, unprotect: Protect) -> Self {
        Self {
            path,
            protect,
            unprotect,
            cached: Mutex::new(None),
        }
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    fn load_or_create(&self) -> io::Result<Vec<u8>> {
        match std::fs::read(&self.path) {
            Ok(sealed) => (self.unprotect)(&sealed),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let key = create_key().into_bytes();
                let sealed = (self.protect)(&key)?;
                if let Some(dir) = self.path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let part = self.path.with_extension("part");
                std::fs::write(&part, sealed)?;
                std::fs::rename(&part, &self.path)?;
                Ok(key)
            }
            Err(error) => Err(error),
        }
    }
}

impl KeyManager for ProtectedKeyFile {
    fn get_key(&self) -> CryptoResult<Vec<u8>> {
        let mut cached = self.cached.lock().map_err(|_| CryptoError::MissingKey)?;
        if let Some(key) = cached.as_ref() {
            return Ok(key.clone());
        }
        let key = self.load_or_create().map_err(|error| {
            log::error!("logins key {}: {error}", self.path.display());
            CryptoError::MissingKey
        })?;
        *cached = Some(key.clone());
        Ok(key)
    }
}

fn saved(login: Login) -> SavedLogin {
    SavedLogin {
        id: login.id,
        origin: login.origin,
        form_action_origin: login.form_action_origin,
        http_realm: login.http_realm,
        username: login.username,
        password: login.password,
        time_created: login.time_created,
        time_last_used: login.time_last_used,
        time_password_changed: login.time_password_changed,
        times_used: login.times_used,
    }
}

fn entry(input: LoginInput) -> LoginEntry {
    LoginEntry {
        origin: input.origin,
        form_action_origin: input.form_action_origin,
        http_realm: input.http_realm,
        username_field: input.username_field,
        password_field: input.password_field,
        username: input.username,
        password: input.password,
    }
}

pub type Callback<T> = Box<dyn FnOnce(T) + Send>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoginsImportStats {
    pub added: usize,
    pub failed: usize,
}

enum Message {
    List {
        reply: Callback<Vec<SavedLogin>>,
    },
    ForOrigin {
        origin: String,
        reply: Callback<Vec<SavedLogin>>,
    },
    Save {
        input: LoginInput,
        reply: Callback<Result<SavedLogin, String>>,
    },
    Update {
        id: String,
        username: String,
        password: String,
        reply: Callback<Result<SavedLogin, String>>,
    },
    Touch {
        id: String,
    },
    Delete {
        id: String,
    },
    DeleteAll,
    Import {
        logins: Vec<ImportedLogin>,
        reply: Callback<LoginsImportStats>,
    },
}

pub struct LoginsService {
    sender: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

fn host_of(origin: &str) -> Option<String> {
    url::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
}

fn matching(store: &LoginStore, origin: &str) -> crate::logins::Result<Vec<SavedLogin>> {
    let Some(host) = host_of(origin) else {
        return Ok(Vec::new());
    };
    let base = if host.parse::<std::net::IpAddr>().is_ok() {
        host.as_str()
    } else {
        psl::domain_str(&host).unwrap_or(&host)
    };
    let mut logins: Vec<SavedLogin> = store
        .get_by_base_domain(base)?
        .into_iter()
        .filter(|l| l.http_realm.is_none())
        .map(saved)
        .collect();
    logins.sort_by_key(|l| (l.origin != origin, -l.time_last_used));
    Ok(logins)
}

fn run(store: &LoginStore, message: Message) {
    match message {
        Message::List { reply } => {
            let mut logins: Vec<SavedLogin> = match store.list() {
                Ok(list) => list.into_iter().map(saved).collect(),
                Err(error) => {
                    log::error!("list logins: {error}");
                    Vec::new()
                }
            };
            logins.sort_by(|a, b| {
                host_of(&a.origin)
                    .cmp(&host_of(&b.origin))
                    .then(a.username.cmp(&b.username))
            });
            reply(logins);
        }
        Message::ForOrigin { origin, reply } => {
            reply(matching(store, &origin).unwrap_or_else(|error| {
                log::error!("logins for origin: {error}");
                Vec::new()
            }));
        }
        Message::Save { input, reply } => {
            reply(
                store
                    .add_or_update(entry(input))
                    .map(saved)
                    .map_err(|e| e.to_string()),
            );
        }
        Message::Update {
            id,
            username,
            password,
            reply,
        } => {
            let result = store.get(&id).and_then(|login| {
                let login = login.ok_or_else(|| crate::logins::Error::NoSuchRecord(id.clone()))?;
                store.update(
                    &id,
                    LoginEntry {
                        origin: login.origin,
                        form_action_origin: login.form_action_origin,
                        http_realm: login.http_realm,
                        username_field: login.username_field,
                        password_field: login.password_field,
                        username,
                        password,
                    },
                )
            });
            reply(result.map(saved).map_err(|e| e.to_string()));
        }
        Message::Touch { id } => {
            if let Err(error) = store.touch(&id) {
                log::warn!("touch login: {error}");
            }
        }
        Message::Delete { id } => {
            if let Err(error) = store.delete(&id) {
                log::error!("delete login: {error}");
            }
        }
        Message::DeleteAll => {
            if let Err(error) = store.delete_all() {
                log::error!("delete all logins: {error}");
            }
        }
        Message::Import { logins, reply } => {
            let entries = logins
                .into_iter()
                .map(|login| LoginEntryWithMeta {
                    meta: LoginMeta {
                        id: login.guid.unwrap_or_default(),
                        time_created: login.time_created,
                        time_password_changed: login.time_password_changed,
                        time_last_used: login.time_last_used,
                        times_used: login.times_used,
                        time_last_breach_alert_dismissed: None,
                    },
                    entry: entry(login.input),
                })
                .collect();
            let stats = match store.add_many_with_meta(entries) {
                Ok(results) => {
                    let added = results
                        .iter()
                        .filter(|r| matches!(r, crate::logins::BulkResultEntry::Success { .. }))
                        .count();
                    LoginsImportStats {
                        added,
                        failed: results.len() - added,
                    }
                }
                Err(error) => {
                    log::error!("import logins: {error}");
                    LoginsImportStats::default()
                }
            };
            reply(stats);
        }
    }
}

impl LoginsService {
    pub fn start(path: PathBuf, key: Arc<ProtectedKeyFile>) -> Self {
        let (sender, receiver) = channel::<Message>();
        let thread = std::thread::Builder::new()
            .name("logins".into())
            .spawn(move || {
                let new_key = !key.exists();
                let encdec = Arc::new(ManagedEncryptorDecryptor::new(key));
                let store = match LoginStore::new(&path, encdec) {
                    Ok(store) => store,
                    Err(error) => {
                        log::error!("open {}: {error}", path.display());
                        return;
                    }
                };
                if new_key && !store.is_empty().unwrap_or(true) {
                    log::warn!("logins key was lost; removing logins that cannot be decrypted");
                    let _ = store.wipe_local();
                }
                for message in receiver {
                    run(&store, message);
                }
                store.shutdown();
            })
            .ok();
        Self {
            sender: Some(sender),
            thread,
        }
    }

    fn send(&self, message: Message) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(message);
        }
    }

    pub fn lister(&self) -> impl Fn(Callback<Vec<SavedLogin>>) + Send + 'static {
        let sender = self.sender.clone();
        move |reply| {
            if let Some(sender) = &sender {
                let _ = sender.send(Message::List { reply });
            }
        }
    }

    pub fn list(&self, reply: impl FnOnce(Vec<SavedLogin>) + Send + 'static) {
        self.send(Message::List {
            reply: Box::new(reply),
        });
    }

    pub fn for_origin(&self, origin: String, reply: impl FnOnce(Vec<SavedLogin>) + Send + 'static) {
        self.send(Message::ForOrigin {
            origin,
            reply: Box::new(reply),
        });
    }

    pub fn save(
        &self,
        input: LoginInput,
        reply: impl FnOnce(Result<SavedLogin, String>) + Send + 'static,
    ) {
        self.send(Message::Save {
            input,
            reply: Box::new(reply),
        });
    }

    pub fn update(
        &self,
        id: String,
        username: String,
        password: String,
        reply: impl FnOnce(Result<SavedLogin, String>) + Send + 'static,
    ) {
        self.send(Message::Update {
            id,
            username,
            password,
            reply: Box::new(reply),
        });
    }

    pub fn touch(&self, id: String) {
        self.send(Message::Touch { id });
    }

    pub fn delete(&self, id: String) {
        self.send(Message::Delete { id });
    }

    pub fn delete_all(&self) {
        self.send(Message::DeleteAll);
    }

    pub fn import(
        &self,
        logins: Vec<ImportedLogin>,
        reply: impl FnOnce(LoginsImportStats) + Send + 'static,
    ) {
        self.send(Message::Import {
            logins,
            reply: Box::new(reply),
        });
    }
}

impl Drop for LoginsService {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(data: &[u8]) -> io::Result<Vec<u8>> {
        Ok(data.to_vec())
    }

    fn reversed(data: &[u8]) -> io::Result<Vec<u8>> {
        Ok(data.iter().rev().copied().collect())
    }

    fn wait<T: Send + 'static>(call: impl FnOnce(Box<dyn FnOnce(T) + Send>)) -> T {
        let (tx, rx) = channel();
        call(Box::new(move |value| {
            let _ = tx.send(value);
        }));
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap()
    }

    fn input(origin: &str, username: &str, password: &str) -> LoginInput {
        LoginInput {
            origin: origin.into(),
            form_action_origin: Some(origin.into()),
            username: username.into(),
            password: password.into(),
            ..Default::default()
        }
    }

    #[test]
    fn saves_finds_updates_and_deletes_logins() {
        let dir = tempfile::tempdir().unwrap();
        let key = Arc::new(ProtectedKeyFile::new(
            dir.path().join("key"),
            reversed,
            reversed,
        ));
        let service = LoginsService::start(dir.path().join("logins.db"), key);
        let saved = wait(|r| service.save(input("https://example.com", "ann", "pw1"), r)).unwrap();
        wait(|r| service.save(input("https://other.org", "bob", "pw2"), r)).unwrap();
        let found = wait(|r| service.for_origin("https://www.example.com".into(), r));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].username, "ann");
        let updated =
            wait(|r| service.update(saved.id.clone(), "ann".into(), "pw3".into(), r)).unwrap();
        assert_eq!(updated.password, "pw3");
        service.delete(saved.id);
        let all = wait(|r| service.list(r));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].origin, "https://other.org");
        let raw = std::fs::read(dir.path().join("key")).unwrap();
        assert!(raw.ends_with(b"{"));
    }

    #[test]
    fn keeps_the_key_between_sessions_and_drops_logins_when_it_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("logins.db");
        let key_path = dir.path().join("key");
        let open = || {
            LoginsService::start(
                db.clone(),
                Arc::new(ProtectedKeyFile::new(key_path.clone(), identity, identity)),
            )
        };
        {
            let service = open();
            wait(|r| service.save(input("https://example.com", "ann", "pw"), r)).unwrap();
        }
        {
            let service = open();
            let all = wait(|r| service.list(r));
            assert_eq!(all[0].password, "pw");
        }
        std::fs::remove_file(&key_path).unwrap();
        let service = open();
        assert!(wait(|r| service.list(r)).is_empty());
    }

    #[test]
    fn imports_firefox_logins_with_their_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let key = Arc::new(ProtectedKeyFile::new(
            dir.path().join("key"),
            identity,
            identity,
        ));
        let service = LoginsService::start(dir.path().join("logins.db"), key);
        let login = ImportedLogin {
            input: input("https://example.com", "ann", "pw"),
            guid: Some("{0b9a0e0e-1111-4c4c-9a9a-123456789abc}".into()),
            time_created: 1_600_000_000_000,
            time_last_used: 1_700_000_000_000,
            time_password_changed: 1_650_000_000_000,
            times_used: 7,
        };
        let bad = ImportedLogin {
            input: input("", "x", "y"),
            ..Default::default()
        };
        let stats = wait(|r| service.import(vec![login, bad], r));
        assert_eq!(
            stats,
            LoginsImportStats {
                added: 1,
                failed: 1
            }
        );
        let all = wait(|r| service.list(r));
        assert_eq!(all[0].times_used, 7);
        assert_eq!(all[0].time_created, 1_600_000_000_000);
    }
}
