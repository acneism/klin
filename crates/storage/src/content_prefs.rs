use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

pub const FULL_ZOOM: &str = "browser.content.full-zoom";

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS groups (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS prefs (
        id INTEGER PRIMARY KEY,
        groupID INTEGER REFERENCES groups(id),
        settingID INTEGER NOT NULL REFERENCES settings(id),
        value BLOB,
        timestamp INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS groups_idx ON groups(name);
    CREATE INDEX IF NOT EXISTS settings_idx ON settings(name);
    CREATE INDEX IF NOT EXISTS prefs_idx ON prefs(timestamp, groupID, settingID);
";

type Values = Vec<(String, f64)>;

enum Message {
    Set {
        group: String,
        setting: String,
        value: Option<f64>,
    },
    GetAll {
        setting: String,
        reply: Box<dyn FnOnce(Values) + Send>,
    },
}

pub struct ContentPrefs {
    tx: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl ContentPrefs {
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("content-prefs".into())
            .spawn(move || run(&path, rx))
            .map_err(|e| log::error!("content-prefs: failed to start thread: {e}"))
            .ok();
        Self {
            tx: Some(tx),
            thread,
        }
    }

    pub fn set(&self, group: &str, setting: &str, value: Option<f64>) {
        self.send(Message::Set {
            group: group.into(),
            setting: setting.into(),
            value,
        });
    }

    pub fn get_all(&self, setting: &str, reply: impl FnOnce(Values) + Send + 'static) {
        self.send(Message::GetAll {
            setting: setting.into(),
            reply: Box::new(reply),
        });
    }

    fn send(&self, message: Message) {
        if let Some(tx) = &self.tx
            && tx.send(message).is_err()
        {
            log::warn!("content-prefs: service is not running");
        }
    }
}

impl Drop for ContentPrefs {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(path: &Path, rx: Receiver<Message>) {
    let db = match open(path) {
        Ok(db) => db,
        Err(e) => {
            log::error!("content-prefs: cannot open {}: {e}", path.display());

            for message in rx {
                if let Message::GetAll { reply, .. } = message {
                    reply(Vec::new());
                }
            }
            return;
        }
    };
    for message in rx {
        let result = match message {
            Message::Set {
                group,
                setting,
                value,
            } => set(&db, &group, &setting, value),
            Message::GetAll { setting, reply } => get_all(&db, &setting).map(reply),
        };
        if let Err(e) = result {
            log::error!("content-prefs: {e}");
        }
    }
}

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;

    db.pragma_update(None, "journal_mode", "WAL")?;
    db.execute_batch(SCHEMA)?;
    Ok(db)
}

fn id_of(db: &Connection, table: &str, name: &str) -> rusqlite::Result<i64> {
    let found = db
        .query_row(
            &format!("SELECT id FROM {table} WHERE name = ?1"),
            [name],
            |r| r.get(0),
        )
        .optional()?;
    match found {
        Some(id) => Ok(id),
        None => {
            db.execute(&format!("INSERT INTO {table} (name) VALUES (?1)"), [name])?;
            Ok(db.last_insert_rowid())
        }
    }
}

fn set(db: &Connection, group: &str, setting: &str, value: Option<f64>) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let group_id = id_of(&tx, "groups", group)?;
    let setting_id = id_of(&tx, "settings", setting)?;
    tx.execute(
        "DELETE FROM prefs WHERE groupID = ?1 AND settingID = ?2",
        params![group_id, setting_id],
    )?;
    if let Some(value) = value {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        tx.execute(
            "INSERT INTO prefs (groupID, settingID, value, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![group_id, setting_id, value, now],
        )?;
    }

    tx.execute(
        "DELETE FROM groups WHERE id NOT IN (SELECT groupID FROM prefs WHERE groupID IS NOT NULL)",
        [],
    )?;
    tx.commit()
}

fn get_all(db: &Connection, setting: &str) -> rusqlite::Result<Values> {
    let mut stmt = db.prepare(
        "SELECT groups.name, prefs.value FROM prefs
         JOIN groups ON groups.id = prefs.groupID
         JOIN settings ON settings.id = prefs.settingID
         WHERE settings.name = ?1",
    )?;
    let rows = stmt.query_map([setting], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn temp_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("klin-storage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("content-prefs.sqlite")
    }

    fn all(prefs: &ContentPrefs) -> Values {
        let (tx, rx) = mpsc::channel();
        prefs.get_all(FULL_ZOOM, move |v| tx.send(v).unwrap());
        let mut v = rx.recv().unwrap();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn values_persist_across_restarts() {
        let path = temp_db("persist");
        {
            let prefs = ContentPrefs::start(path.clone());
            prefs.set("example.com", FULL_ZOOM, Some(1.2));
            prefs.set("servo.org", FULL_ZOOM, Some(0.9));
            prefs.set("example.com", FULL_ZOOM, Some(1.33));
        }
        let prefs = ContentPrefs::start(path.clone());
        assert_eq!(
            all(&prefs),
            [
                ("example.com".to_owned(), 1.33),
                ("servo.org".to_owned(), 0.9)
            ]
        );
        prefs.set("servo.org", FULL_ZOOM, None);
        assert_eq!(all(&prefs), [("example.com".to_owned(), 1.33)]);
        drop(prefs);

        let db = Connection::open(&path).unwrap();
        let mode: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let groups: i64 = db
            .query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0))
            .unwrap();
        assert_eq!(groups, 1);
    }
}
