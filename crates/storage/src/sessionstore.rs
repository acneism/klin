use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use klin_shared::Url;
use klin_shared::session::{ClosedTabState, EntryState, Session, TabState, WindowState};
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"mozLz40\0";
const STATE_RUNNING: &str = "running";
const STATE_STOPPED: &str = "stopped";

pub fn compress_mozlz4(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len() / 2);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&lz4_flex::block::compress(data));
    out
}

pub fn decompress_mozlz4(data: &[u8]) -> io::Result<Vec<u8>> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_owned());
    if data.len() < 12 || &data[..8] != MAGIC {
        return Err(bad("not a mozlz4 file"));
    }
    let size = u32::from_le_bytes(data[8..12].try_into().expect("4 bytes")) as usize;
    lz4_flex::block::decompress(&data[12..], size).map_err(|e| bad(&e.to_string()))
}

#[derive(Serialize, Deserialize, Default)]
struct FileState {
    #[serde(default)]
    version: Vec<serde_json::Value>,
    #[serde(default)]
    windows: Vec<FileWindow>,
    #[serde(rename = "selectedWindow", default)]
    selected_window: usize,
    #[serde(rename = "_closedWindows", default)]
    closed_windows: Vec<serde_json::Value>,
    #[serde(default)]
    session: FileSession,
    #[serde(default)]
    global: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileSession {
    #[serde(rename = "lastUpdate", default)]
    last_update: u64,
    #[serde(rename = "startTime", default)]
    start_time: u64,
    #[serde(rename = "recentCrashes", default)]
    recent_crashes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileWindow {
    #[serde(default)]
    tabs: Vec<FileTab>,
    #[serde(default)]
    selected: usize,
    #[serde(rename = "_closedTabs", default)]
    closed_tabs: Vec<FileClosedTab>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileTab {
    #[serde(default)]
    entries: Vec<FileEntry>,
    #[serde(default)]
    index: usize,
    #[serde(rename = "lastAccessed", default)]
    last_accessed: u64,
    #[serde(default)]
    hidden: bool,
    #[serde(rename = "userContextId", default)]
    user_context_id: u32,
    #[serde(default)]
    attributes: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileEntry {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default = "yes")]
    persist: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Default)]
struct FileClosedTab {
    #[serde(default)]
    state: FileTab,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pos: usize,
    #[serde(rename = "closedAt", default)]
    closed_at: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn to_file_tab(tab: &TabState) -> FileTab {
    FileTab {
        entries: tab
            .entries
            .iter()
            .map(|e| FileEntry {
                url: e.url.to_string(),
                title: e.title.clone(),
                persist: true,
            })
            .collect(),
        index: tab.index + 1,
        last_accessed: now_ms(),
        ..FileTab::default()
    }
}

fn from_file_tab(tab: &FileTab) -> Option<TabState> {
    let entries: Vec<EntryState> = tab
        .entries
        .iter()
        .filter_map(|e| {
            Some(EntryState {
                url: Url::parse(&e.url).ok()?,
                title: e.title.clone(),
            })
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let index = tab.index.saturating_sub(1).min(entries.len() - 1);
    Some(TabState { entries, index })
}

fn to_file(session: &Session, state: &str, start_time: u64) -> FileState {
    FileState {
        version: vec!["sessionrestore".into(), 1.into()],
        windows: session
            .windows
            .iter()
            .map(|w| FileWindow {
                tabs: w.tabs.iter().map(to_file_tab).collect(),
                selected: w.selected + 1,
                closed_tabs: w
                    .closed_tabs
                    .iter()
                    .map(|c| FileClosedTab {
                        state: to_file_tab(&c.tab),
                        title: c.tab.current().map(|e| e.title.clone()).unwrap_or_default(),
                        pos: c.pos,
                        closed_at: c.closed_at_ms,
                    })
                    .collect(),
            })
            .collect(),
        selected_window: session.selected_window + 1,
        closed_windows: Vec::new(),
        session: FileSession {
            last_update: now_ms(),
            start_time,
            recent_crashes: 0,
            state: Some(state.to_owned()),
        },
        global: serde_json::Map::new(),
    }
}

fn from_file(file: &FileState) -> Session {
    let windows = file
        .windows
        .iter()
        .map(|w| {
            let tabs: Vec<TabState> = w
                .tabs
                .iter()
                .filter(|t| !t.hidden)
                .filter_map(from_file_tab)
                .collect();
            WindowState {
                selected: w
                    .selected
                    .saturating_sub(1)
                    .min(tabs.len().saturating_sub(1)),
                tabs,
                closed_tabs: w
                    .closed_tabs
                    .iter()
                    .filter_map(|c| {
                        Some(ClosedTabState {
                            tab: from_file_tab(&c.state)?,
                            pos: c.pos,
                            closed_at_ms: c.closed_at,
                        })
                    })
                    .collect(),
            }
        })
        .filter(|w| !w.tabs.is_empty())
        .collect::<Vec<_>>();
    let selected_window = file
        .selected_window
        .saturating_sub(1)
        .min(windows.len().saturating_sub(1));
    Session {
        windows,
        selected_window,
    }
}

pub fn read_session(path: &Path) -> io::Result<(Session, bool)> {
    let bytes = fs::read(path)?;
    let json = decompress_mozlz4(&bytes)?;
    let file: FileState =
        serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let running = file.session.state.as_deref() == Some(STATE_RUNNING);
    Ok((from_file(&file), running))
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        io::Write::write_all(&mut f, data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[derive(Debug, PartialEq)]
pub enum StartupSession {
    Restore { session: Session, after_crash: bool },
    Fresh,
}

#[derive(Clone)]
pub struct SessionPaths {
    pub main: PathBuf,
    pub recovery: PathBuf,
    pub recovery_backup: PathBuf,
    pub previous: PathBuf,
}

impl SessionPaths {
    pub fn new(profile_dir: &Path) -> Self {
        let backups = profile_dir.join("sessionstore-backups");
        Self {
            main: profile_dir.join("sessionstore.jsonlz4"),
            recovery: backups.join("recovery.jsonlz4"),
            recovery_backup: backups.join("recovery.baklz4"),
            previous: backups.join("previous.jsonlz4"),
        }
    }
}

pub fn startup_session(
    paths: &SessionPaths,
    startup_page: i64,
    resume_from_crash: bool,
) -> StartupSession {
    let recovery = read_session(&paths.recovery).or_else(|_| read_session(&paths.recovery_backup));
    let result = match recovery {
        Ok((session, true)) if resume_from_crash && !session.windows.is_empty() => {
            StartupSession::Restore {
                session,
                after_crash: true,
            }
        }
        _ => match read_session(&paths.main) {
            Ok((session, _)) if startup_page == 3 && !session.windows.is_empty() => {
                StartupSession::Restore {
                    session,
                    after_crash: false,
                }
            }
            _ => StartupSession::Fresh,
        },
    };

    if paths.main.exists() {
        if let Some(dir) = paths.previous.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(e) = fs::rename(&paths.main, &paths.previous) {
            log::warn!("sessionstore: cannot move {}: {e}", paths.main.display());
        }
    }
    result
}

enum Message {
    Running(Session),
    Stopped(Session),
}

pub struct SessionWriter {
    tx: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl SessionWriter {
    pub fn start(paths: SessionPaths) -> Self {
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("sessionstore".into())
            .spawn(move || run(paths, rx))
            .map_err(|e| log::error!("sessionstore: failed to start thread: {e}"))
            .ok();
        Self {
            tx: Some(tx),
            thread,
        }
    }

    pub fn save(&self, session: Session) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Message::Running(session));
        }
    }

    pub fn finish(mut self, session: Session) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Message::Stopped(session));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SessionWriter {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(paths: SessionPaths, rx: Receiver<Message>) {
    let start_time = now_ms();
    for message in rx {
        let result = match message {
            Message::Running(session) => write_running(&paths, &session, start_time),
            Message::Stopped(session) => write_stopped(&paths, &session, start_time),
        };
        if let Err(e) = result {
            log::error!("sessionstore: {e}");
        }
    }
}

fn encode(session: &Session, state: &str, start_time: u64) -> io::Result<Vec<u8>> {
    let json =
        serde_json::to_vec(&to_file(session, state, start_time)).map_err(io::Error::other)?;
    Ok(compress_mozlz4(&json))
}

fn write_running(paths: &SessionPaths, session: &Session, start_time: u64) -> io::Result<()> {
    let data = encode(session, STATE_RUNNING, start_time)?;
    if paths.recovery.exists() {
        let _ = fs::rename(&paths.recovery, &paths.recovery_backup);
    }
    write_atomic(&paths.recovery, &data)
}

fn write_stopped(paths: &SessionPaths, session: &Session, start_time: u64) -> io::Result<()> {
    let data = encode(session, STATE_STOPPED, start_time)?;
    write_atomic(&paths.main, &data)?;
    for p in [&paths.recovery, &paths.recovery_backup] {
        match fs::remove_file(p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let url = |s: &str| Url::parse(s).unwrap();
        Session {
            windows: vec![WindowState {
                tabs: vec![
                    TabState {
                        entries: vec![
                            EntryState {
                                url: url("https://a.example/0"),
                                title: String::new(),
                            },
                            EntryState {
                                url: url("https://a.example/"),
                                title: "A".into(),
                            },
                        ],
                        index: 1,
                    },
                    TabState {
                        entries: vec![EntryState {
                            url: url("https://b.example/"),
                            title: "B".into(),
                        }],
                        index: 0,
                    },
                ],
                selected: 1,
                closed_tabs: vec![ClosedTabState {
                    tab: TabState {
                        entries: vec![EntryState {
                            url: url("https://c.example/"),
                            title: "C".into(),
                        }],
                        index: 0,
                    },
                    pos: 2,
                    closed_at_ms: 1_700_000_000_000,
                }],
            }],
            selected_window: 0,
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("klin-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mozlz4_round_trip() {
        let data = b"{\"windows\":[]} repeated repeated repeated repeated".repeat(20);
        let packed = compress_mozlz4(&data);
        assert_eq!(&packed[..8], MAGIC);
        assert_eq!(decompress_mozlz4(&packed).unwrap(), data);
        assert!(decompress_mozlz4(b"garbage").is_err());
    }

    #[test]
    fn json_uses_firefox_layout() {
        let json = serde_json::to_value(to_file(&session(), STATE_RUNNING, 1)).unwrap();
        assert_eq!(json["version"], serde_json::json!(["sessionrestore", 1]));
        assert_eq!(json["windows"][0]["selected"], 2);
        assert_eq!(json["windows"][0]["tabs"][0]["index"], 2);
        assert_eq!(
            json["windows"][0]["tabs"][0]["entries"][1]["url"],
            "https://a.example/"
        );
        assert_eq!(
            json["windows"][0]["_closedTabs"][0]["state"]["entries"][0]["title"],
            "C"
        );
        assert_eq!(json["session"]["state"], "running");
        assert_eq!(from_file(&serde_json::from_value(json).unwrap()), session());
    }

    #[test]
    fn crash_restores_and_clean_exit_follows_startup_page() {
        let dir = temp("startup");
        let paths = SessionPaths::new(&dir);

        let writer = SessionWriter::start(paths.clone());
        writer.save(session());
        drop(writer);
        assert!(paths.recovery.exists());
        assert_eq!(
            startup_session(&paths, 1, true),
            StartupSession::Restore {
                session: session(),
                after_crash: true
            }
        );
        assert_eq!(startup_session(&paths, 1, false), StartupSession::Fresh);

        let writer = SessionWriter::start(paths.clone());
        writer.save(session());
        writer.finish(session());
        assert!(!paths.recovery.exists() && !paths.recovery_backup.exists());
        assert!(paths.main.exists());

        assert_eq!(startup_session(&paths, 1, true), StartupSession::Fresh);
        assert!(!paths.main.exists() && paths.previous.exists());

        SessionWriter::start(paths.clone()).finish(session());
        assert_eq!(
            startup_session(&paths, 3, true),
            StartupSession::Restore {
                session: session(),
                after_crash: false
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
