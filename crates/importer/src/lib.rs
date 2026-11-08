use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use klin_profile::Ini;
use klin_shared::Url;
use klin_shared::import::{
    BookmarkRoot, ImportedBookmark, ImportedCookie, ImportedData, ImportedPlace, ImportedRoot,
    ImportedVisit, SameSite,
};
use rusqlite::{Connection, OpenFlags};

pub mod nss;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("профиль Firefox не найден")]
    NoProfile,
    #[error("ошибка чтения {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ошибка базы {path}: {source}")]
    Sql {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
}

pub fn firefox_base_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Mozilla").join("Firefox"))
    } else {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".mozilla").join("firefox"))
    }
}

pub fn default_profile(base: &Path) -> Option<PathBuf> {
    let read = |name: &str| {
        fs::read_to_string(base.join(name))
            .ok()
            .map(|t| Ini::parse(&t))
    };
    let resolve = |ini: &Ini, section: &str, path: &str| -> PathBuf {
        let relative = ini.get(section, "IsRelative") != Some("0");
        let path = path.replace('\\', "/");
        if relative {
            path.split('/').fold(base.to_path_buf(), |p, s| p.join(s))
        } else {
            PathBuf::from(path)
        }
    };
    if let Some(installs) = read("installs.ini") {
        for section in installs.sections() {
            if let Some(path) = installs.get(section, "Default") {
                let dir = path
                    .replace('\\', "/")
                    .split('/')
                    .fold(base.to_path_buf(), |p, s| p.join(s));
                if dir.is_dir() {
                    return Some(dir);
                }
            }
        }
    }
    let profiles = read("profiles.ini")?;
    let sections: Vec<String> = profiles.sections().map(str::to_owned).collect();
    for section in sections.iter().filter(|s| s.starts_with("Install")) {
        if let Some(path) = profiles.get(section, "Default") {
            let dir = path
                .replace('\\', "/")
                .split('/')
                .fold(base.to_path_buf(), |p, s| p.join(s));
            if dir.is_dir() {
                return Some(dir);
            }
        }
    }
    let candidates: Vec<&String> = sections
        .iter()
        .filter(|s| s.starts_with("Profile"))
        .collect();
    let chosen = candidates
        .iter()
        .find(|s| profiles.get(s, "Default") == Some("1"))
        .or_else(|| candidates.first())?;
    let path = profiles.get(chosen, "Path")?;
    Some(resolve(&profiles, chosen, path))
}

pub fn import_profile(dir: &Path, temp: &Path) -> Result<ImportedData, ImportError> {
    let mut data = ImportedData::default();
    if dir.join("places.sqlite").exists() {
        let conn = open_copy(dir, "places.sqlite", temp)?;
        let path = dir.join("places.sqlite");
        let sql = |source| ImportError::Sql {
            path: path.clone(),
            source,
        };
        data.places = read_history(&conn).map_err(sql)?;
        data.bookmarks = read_bookmarks(&conn).map_err(sql)?;
    }
    if dir.join("cookies.sqlite").exists() {
        let conn = open_copy(dir, "cookies.sqlite", temp)?;
        let path = dir.join("cookies.sqlite");
        data.cookies = read_cookies(&conn).map_err(|source| ImportError::Sql { path, source })?;
    }
    if dir.join("logins.json").exists() && dir.join("key4.db").exists() {
        match copy_key4(dir, temp).and_then(|key4| read_logins(dir, &key4)) {
            Ok(logins) => data.logins = logins,
            Err(error) => data.logins_error = Some(error.to_string()),
        }
    }
    let _ = fs::remove_dir_all(temp);
    Ok(data)
}

fn copy_key4(dir: &Path, temp: &Path) -> Result<PathBuf, nss::NssError> {
    fs::create_dir_all(temp).map_err(|_| nss::NssError::Format)?;
    for suffix in ["", "-wal"] {
        let from = dir.join(format!("key4.db{suffix}"));
        if from.exists() {
            fs::copy(&from, temp.join(format!("key4.db{suffix}")))
                .map_err(|_| nss::NssError::Format)?;
        }
    }
    Ok(temp.join("key4.db"))
}

pub fn read_logins(
    dir: &Path,
    key4: &Path,
) -> Result<Vec<klin_shared::logins::ImportedLogin>, nss::NssError> {
    let key = nss::logins_key(key4, "")?;
    let text = fs::read_to_string(dir.join("logins.json")).map_err(|_| nss::NssError::Format)?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| nss::NssError::Format)?;
    let mut logins = Vec::new();
    for entry in json
        .get("logins")
        .and_then(|l| l.as_array())
        .into_iter()
        .flatten()
    {
        let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_owned);
        let number = |key: &str| entry.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
        let (Some(origin), Some(username), Some(password)) = (
            text("hostname"),
            text("encryptedUsername"),
            text("encryptedPassword"),
        ) else {
            continue;
        };
        let (Ok(username), Ok(password)) = (key.decrypt(&username), key.decrypt(&password)) else {
            log::warn!("cannot decrypt a Firefox login for {origin}");
            continue;
        };
        logins.push(klin_shared::logins::ImportedLogin {
            input: klin_shared::logins::LoginInput {
                origin,
                form_action_origin: text("formSubmitURL"),
                http_realm: text("httpRealm"),
                username_field: text("usernameField").unwrap_or_default(),
                password_field: text("passwordField").unwrap_or_default(),
                username,
                password,
            },
            guid: text("guid"),
            time_created: number("timeCreated"),
            time_last_used: number("timeLastUsed"),
            time_password_changed: number("timePasswordChanged"),
            times_used: number("timesUsed"),
        });
    }
    Ok(logins)
}

fn open_copy(dir: &Path, name: &str, temp: &Path) -> Result<Connection, ImportError> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| ImportError::Io { path, source }
    };
    fs::create_dir_all(temp).map_err(io(temp))?;
    for suffix in ["", "-wal"] {
        let from = dir.join(format!("{name}{suffix}"));
        if from.exists() {
            fs::copy(&from, temp.join(format!("{name}{suffix}"))).map_err(io(&from))?;
        }
    }
    let copy = temp.join(name);
    Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|source| ImportError::Sql { path: copy, source })
}

fn importable(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "file" | "ftp")
}

fn read_history(conn: &Connection) -> rusqlite::Result<Vec<ImportedPlace>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.url, p.title, v.visit_date, v.visit_type
         FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id
         ORDER BY p.id, v.visit_date",
    )?;
    let mut rows = stmt.query([])?;
    let mut places: Vec<ImportedPlace> = Vec::new();
    let mut last_id = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let url: String = row.get(1)?;
        let title: Option<String> = row.get(2)?;
        let date: i64 = row.get(3)?;
        let visit_type: i64 = row.get(4)?;
        let visit = ImportedVisit {
            at_ms: (date.max(0) / 1000) as u64,
            visit_type: visit_type.clamp(0, 255) as u8,
        };
        if last_id == Some(id) {
            if let Some(place) = places.last_mut() {
                place.visits.push(visit);
            }
            continue;
        }
        let Ok(url) = Url::parse(&url) else { continue };
        if !importable(&url) {
            continue;
        }
        last_id = Some(id);
        places.push(ImportedPlace {
            url,
            title: title.filter(|t| !t.is_empty()),
            visits: vec![visit],
        });
    }
    Ok(places)
}

struct Row {
    id: i64,
    kind: i64,
    parent: i64,
    title: Option<String>,
    added: i64,
    guid: String,
    url: Option<String>,
}

fn read_bookmarks(conn: &Connection) -> rusqlite::Result<Vec<ImportedRoot>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.type, b.parent, b.title, b.dateAdded, b.guid, p.url
         FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk
         ORDER BY b.parent, b.position",
    )?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                kind: r.get(1)?,
                parent: r.get(2)?,
                title: r.get(3)?,
                added: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                guid: r.get(5)?,
                url: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut children: HashMap<i64, Vec<&Row>> = HashMap::new();
    for row in &rows {
        children.entry(row.parent).or_default().push(row);
    }
    fn build(id: i64, children: &HashMap<i64, Vec<&Row>>, depth: usize) -> Vec<ImportedBookmark> {
        if depth > 64 {
            return Vec::new();
        }
        children
            .get(&id)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|row| {
                        let added_ms = (row.added.max(0) / 1000) as u64;
                        match row.kind {
                            1 => {
                                let url =
                                    Url::parse(row.url.as_deref()?).ok().filter(importable)?;
                                Some(ImportedBookmark::Bookmark {
                                    url,
                                    title: row.title.clone(),
                                    added_ms,
                                })
                            }
                            2 => Some(ImportedBookmark::Folder {
                                title: row.title.clone().unwrap_or_default(),
                                added_ms,
                                children: build(row.id, children, depth + 1),
                            }),
                            3 => Some(ImportedBookmark::Separator),
                            _ => None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    let root_for = |guid: &str| match guid {
        "menu________" => Some(BookmarkRoot::Menu),
        "toolbar_____" => Some(BookmarkRoot::Toolbar),
        "unfiled_____" => Some(BookmarkRoot::Unfiled),
        "mobile______" => Some(BookmarkRoot::Mobile),
        _ => None,
    };
    Ok(rows
        .iter()
        .filter_map(|row| {
            let root = root_for(&row.guid)?;
            Some(ImportedRoot {
                root,
                children: build(row.id, &children, 0),
            })
        })
        .filter(|r| !r.children.is_empty())
        .collect())
}

fn read_cookies(conn: &Connection) -> rusqlite::Result<Vec<ImportedCookie>> {
    let mut stmt = conn.prepare(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite
         FROM moz_cookies WHERE originAttributes = ''",
    )?;
    let cookies = stmt
        .query_map([], |r| {
            let expiry: i64 = r.get(4)?;
            let expires_ms = match expiry {
                e if e <= 0 => None,
                e if e > 100_000_000_000 => Some(e as u64),
                e => Some(e as u64 * 1000),
            };
            Ok(ImportedCookie {
                name: r.get(0)?,
                value: r.get(1)?,
                host: r.get(2)?,
                path: r.get(3)?,
                expires_ms,
                secure: r.get::<_, i64>(5)? != 0,
                http_only: r.get::<_, i64>(6)? != 0,
                same_site: match r.get::<_, i64>(7)? {
                    0 => SameSite::None,
                    2 => SameSite::Strict,
                    _ => SameSite::Lax,
                },
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("klin-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_firefox_profile(dir: &Path) {
        let places = Connection::open(dir.join("places.sqlite")).unwrap();
        places
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR,
                    rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL,
                    typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL,
                    last_visit_date INTEGER, guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL,
                    url_hash INTEGER DEFAULT 0 NOT NULL);
                 CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, from_visit INTEGER,
                    place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER);
                 CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL,
                    parent INTEGER, position INTEGER, title LONGVARCHAR, keyword_id INTEGER,
                    folder_type TEXT, dateAdded INTEGER, lastModified INTEGER, guid TEXT,
                    syncStatus INTEGER NOT NULL DEFAULT 0, syncChangeCounter INTEGER NOT NULL DEFAULT 1);
                 INSERT INTO moz_places (id, url, title) VALUES
                    (1, 'https://servo.org/', 'Servo'), (2, 'https://example.com/', NULL),
                    (3, 'place:sort=8&maxResults=10', 'Most Visited');
                 INSERT INTO moz_historyvisits (place_id, visit_date, visit_type) VALUES
                    (1, 1700000000000000, 2), (1, 1700000100000000, 1), (2, 1700000200000000, 1);
                 INSERT INTO moz_bookmarks (id, type, fk, parent, position, title, dateAdded, guid) VALUES
                    (1, 2, NULL, 0, 0, '', 0, 'root________'),
                    (2, 2, NULL, 1, 0, 'menu', 0, 'menu________'),
                    (3, 2, NULL, 1, 1, 'toolbar', 0, 'toolbar_____'),
                    (5, 2, NULL, 1, 3, 'unfiled', 0, 'unfiled_____'),
                    (10, 1, 1, 3, 0, 'Servo', 1700000000000000, 'aaaaaaaaaaaa'),
                    (11, 2, NULL, 3, 1, 'Папка', 1700000000000000, 'bbbbbbbbbbbb'),
                    (12, 1, 2, 11, 0, 'Example', 1700000000000000, 'cccccccccccc'),
                    (13, 3, NULL, 11, 1, NULL, 0, 'dddddddddddd'),
                    (14, 1, 3, 2, 0, 'Most Visited', 0, 'eeeeeeeeeeee');",
            )
            .unwrap();
        let cookies = Connection::open(dir.join("cookies.sqlite")).unwrap();
        cookies
            .execute_batch(
                "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '',
                    name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER,
                    creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0,
                    sameSite INTEGER DEFAULT 0, rawSameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0);
                 INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, isSecure, isHttpOnly, sameSite) VALUES
                    ('', 'session', 'abc', '.servo.org', '/', 1900000000, 1, 1, 1),
                    ('', 'ms', 'x', 'example.com', '/', 1900000000000, 0, 0, 2),
                    ('^userContextId=1', 'container', 'y', 'example.com', '/', 1900000000, 0, 0, 0);",
            )
            .unwrap();
    }

    #[test]
    fn reads_history_bookmarks_and_cookies() {
        let dir = temp("profile");
        fake_firefox_profile(&dir);
        let data = import_profile(&dir, &dir.join("copy")).unwrap();

        assert_eq!(data.places.len(), 2);
        assert_eq!(data.places[0].url.as_str(), "https://servo.org/");
        assert_eq!(data.places[0].visits.len(), 2);
        assert_eq!(
            data.places[0].visits[0],
            ImportedVisit {
                at_ms: 1_700_000_000_000,
                visit_type: 2
            }
        );
        assert_eq!(data.places[1].title, None);

        let toolbar = data
            .bookmarks
            .iter()
            .find(|r| r.root == BookmarkRoot::Toolbar)
            .unwrap();
        assert_eq!(toolbar.children.len(), 2);
        let ImportedBookmark::Folder {
            title, children, ..
        } = &toolbar.children[1]
        else {
            panic!()
        };
        assert_eq!(title, "Папка");
        assert_eq!(children.len(), 2);
        assert_eq!(children[1], ImportedBookmark::Separator);
        assert!(!data.bookmarks.iter().any(|r| r.root == BookmarkRoot::Menu));

        assert_eq!(data.cookies.len(), 2);
        let session = data.cookies.iter().find(|c| c.name == "session").unwrap();
        assert_eq!(session.expires_ms, Some(1_900_000_000_000));
        assert!(session.secure && session.http_only);
        assert_eq!(session.same_site, SameSite::Lax);
        let ms = data.cookies.iter().find(|c| c.name == "ms").unwrap();
        assert_eq!(ms.expires_ms, Some(1_900_000_000_000));
        assert_eq!(ms.same_site, SameSite::Strict);
        assert!(!dir.join("copy").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_default_profile() {
        let base = temp("base");
        fs::create_dir_all(base.join("Profiles/abc.default-release")).unwrap();
        fs::create_dir_all(base.join("Profiles/old.default")).unwrap();
        fs::write(
            base.join("profiles.ini"),
            "[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/old.default\nDefault=1\n\n\
             [Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/abc.default-release\n\n\
             [General]\nStartWithLastProfile=1\nVersion=2\n",
        )
        .unwrap();
        assert_eq!(
            default_profile(&base),
            Some(base.join("Profiles").join("old.default"))
        );
        fs::write(
            base.join("installs.ini"),
            "[308046B0AF4A39CB]\nDefault=Profiles/abc.default-release\nLocked=1\n",
        )
        .unwrap();
        assert_eq!(
            default_profile(&base),
            Some(base.join("Profiles").join("abc.default-release"))
        );
        fs::remove_dir_all(base).unwrap();
    }
}
