use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use klin_shared::bookmarks::BookmarkNode;
use klin_shared::import::{BookmarkRoot, ImportedBookmark, ImportedPlace, ImportedRoot};
use url::Url;

use crate::places::api::matcher::{self, SearchParams};
use crate::places::storage::bookmarks::{
    self, BookmarkPosition, BookmarkRootGuid, InsertableBookmark, InsertableFolder, InsertableItem,
    InsertableSeparator, fetch,
};
use crate::places::storage::history;
use crate::places::types::{VisitTransitionSet, VisitType};
use crate::places::{ConnectionType, PlacesApi, PlacesDb, VisitObservation};
use crate::support::types::Timestamp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub url: Url,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: Url,
    pub title: Option<String>,
    pub visited_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Link,
    Typed,
    Bookmark,
    Reload,
}

type Callback<T> = Box<dyn FnOnce(T) + Send>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub pages: usize,
    pub visits: usize,
    pub bookmarks: usize,
}

enum Message {
    ClearHistory,
    Toolbar {
        reply: Callback<Vec<BookmarkNode>>,
    },
    Import {
        places: Vec<ImportedPlace>,
        bookmarks: Vec<ImportedRoot>,
        reply: Callback<ImportStats>,
    },
    Visit {
        url: Url,
        transition: Transition,
        referrer: Option<Url>,
    },
    Title {
        url: Url,
        title: String,
    },
    Search {
        query: String,
        limit: u32,
        reply: Callback<Vec<Suggestion>>,
    },
    AcceptSuggestion {
        query: String,
        url: Url,
    },
    IsBookmarked {
        url: Url,
        reply: Callback<bool>,
    },
    AddBookmark {
        url: Url,
        title: Option<String>,
    },
    RemoveBookmarks {
        url: Url,
    },
    RecentHistory {
        limit: usize,
        reply: Callback<Vec<HistoryEntry>>,
    },
}

pub struct PlacesService {
    tx: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl PlacesService {
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("places".into())
            .spawn(move || run(path, rx))
            .map_err(|e| log::error!("places: failed to start thread: {e}"))
            .ok();
        Self {
            tx: Some(tx),
            thread,
        }
    }

    fn send(&self, message: Message) {
        if let Some(tx) = &self.tx
            && tx.send(message).is_err()
        {
            log::warn!("places: service is not running");
        }
    }

    pub fn record_visit(&self, url: Url, transition: Transition, referrer: Option<Url>) {
        self.send(Message::Visit {
            url,
            transition,
            referrer,
        });
    }

    pub fn record_title(&self, url: Url, title: String) {
        self.send(Message::Title { url, title });
    }

    pub fn search(
        &self,
        query: String,
        limit: u32,
        reply: impl FnOnce(Vec<Suggestion>) + Send + 'static,
    ) {
        self.send(Message::Search {
            query,
            limit,
            reply: Box::new(reply),
        });
    }

    pub fn accept_suggestion(&self, query: String, url: Url) {
        self.send(Message::AcceptSuggestion { query, url });
    }

    pub fn is_bookmarked(&self, url: Url, reply: impl FnOnce(bool) + Send + 'static) {
        self.send(Message::IsBookmarked {
            url,
            reply: Box::new(reply),
        });
    }

    pub fn add_bookmark(&self, url: Url, title: Option<String>) {
        self.send(Message::AddBookmark { url, title });
    }

    pub fn remove_bookmarks(&self, url: Url) {
        self.send(Message::RemoveBookmarks { url });
    }

    pub fn import(
        &self,
        places: Vec<ImportedPlace>,
        bookmarks: Vec<ImportedRoot>,
        reply: impl FnOnce(ImportStats) + Send + 'static,
    ) {
        self.send(Message::Import {
            places,
            bookmarks,
            reply: Box::new(reply),
        });
    }

    pub fn bookmarks_toolbar(&self, reply: impl FnOnce(Vec<BookmarkNode>) + Send + 'static) {
        self.send(Message::Toolbar {
            reply: Box::new(reply),
        });
    }

    pub fn clear_history(&self) {
        self.send(Message::ClearHistory);
    }

    pub fn recent_history(
        &self,
        limit: usize,
        reply: impl FnOnce(Vec<HistoryEntry>) + Send + 'static,
    ) {
        self.send(Message::RecentHistory {
            limit,
            reply: Box::new(reply),
        });
    }
}

impl Drop for PlacesService {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(path: PathBuf, rx: Receiver<Message>) {
    let db = PlacesApi::new(&path).and_then(|api| api.open_connection(ConnectionType::ReadWrite));
    let mut db = match db {
        Ok(db) => db,
        Err(e) => {
            log::error!("places: cannot open {}: {e}", path.display());

            for message in rx {
                match message {
                    Message::Search { reply, .. } => reply(Vec::new()),
                    Message::IsBookmarked { reply, .. } => reply(false),
                    Message::RecentHistory { reply, .. } => reply(Vec::new()),
                    Message::Import { reply, .. } => reply(ImportStats::default()),
                    Message::Toolbar { reply } => reply(Vec::new()),
                    _ => {}
                }
            }
            return;
        }
    };
    for message in rx {
        if let Err(e) = handle(&mut db, message) {
            log::warn!("places: {e}");
        }
    }
}

fn handle(db: &mut PlacesDb, message: Message) -> crate::places::Result<()> {
    match message {
        Message::Visit {
            url,
            transition,
            referrer,
        } => {
            let visit_type = match transition {
                Transition::Link => VisitType::Link,
                Transition::Typed => VisitType::Typed,
                Transition::Bookmark => VisitType::Bookmark,
                Transition::Reload => VisitType::Reload,
            };
            let observation = VisitObservation::new(url)
                .with_visit_type(visit_type)
                .with_referrer(referrer);
            crate::places::apply_observation(db, observation)?;
        }
        Message::Title { url, title } => {
            let observation = VisitObservation::new(url).with_title(title);
            crate::places::apply_observation(db, observation)?;
        }
        Message::Search {
            query,
            limit,
            reply,
        } => {
            let mut results = matcher::search_frecent(
                db,
                SearchParams {
                    search_string: query,
                    limit,
                },
            )?;

            results.sort_by_key(|r| std::cmp::Reverse(r.frecency));
            reply(
                results
                    .into_iter()
                    .map(|r| Suggestion {
                        url: r.url,
                        title: r.title,
                    })
                    .collect(),
            );
        }
        Message::AcceptSuggestion { query, url } => matcher::accept_result(db, &query, &url)?,
        Message::IsBookmarked { url, reply } => {
            reply(!fetch::fetch_bookmarks_by_url(db, &url)?.is_empty());
        }
        Message::AddBookmark { url, title } => {
            if fetch::fetch_bookmarks_by_url(db, &url)?.is_empty() {
                let bookmark = InsertableBookmark {
                    parent_guid: BookmarkRootGuid::Unfiled.as_guid(),
                    position: BookmarkPosition::Append,
                    date_added: None,
                    last_modified: None,
                    guid: None,
                    url,
                    title,
                };
                bookmarks::insert_bookmark(db, InsertableItem::from(bookmark))?;
            }
        }
        Message::RemoveBookmarks { url } => {
            for bookmark in fetch::fetch_bookmarks_by_url(db, &url)? {
                bookmarks::delete_bookmark(db, &bookmark.guid)?;
            }
        }
        Message::Import {
            places,
            bookmarks,
            reply,
        } => {
            let mut stats = ImportStats::default();
            import_history(db, places, &mut stats)?;
            for root in bookmarks {
                let parent = match root.root {
                    BookmarkRoot::Menu => BookmarkRootGuid::Menu,
                    BookmarkRoot::Toolbar => BookmarkRootGuid::Toolbar,
                    BookmarkRoot::Unfiled => BookmarkRootGuid::Unfiled,
                    BookmarkRoot::Mobile => BookmarkRootGuid::Mobile,
                }
                .as_guid();
                stats.bookmarks += import_bookmarks(db, &parent, root.children)?;
            }
            reply(stats);
        }
        Message::ClearHistory => history::delete_everything(db)?,
        Message::Toolbar { reply } => {
            let tree = fetch::fetch_tree_with_depth(
                db,
                &BookmarkRootGuid::Toolbar.as_guid(),
                &bookmarks::json_tree::FetchDepth::Deepest,
            )?;
            let nodes = match tree {
                Some(fetch::Item::Folder { f }) => f
                    .child_nodes
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(to_node)
                    .collect(),
                _ => Vec::new(),
            };
            reply(nodes);
        }
        Message::RecentHistory { limit, reply } => {
            let infos = history::get_visit_infos(
                db,
                Timestamp(0),
                Timestamp::now(),
                VisitTransitionSet::empty(),
            )?;
            let entries = infos
                .into_iter()
                .rev()
                .take(limit)
                .map(|i| HistoryEntry {
                    url: i.url,
                    title: i.title,
                    visited_ms: i.timestamp.0,
                })
                .collect();
            reply(entries);
        }
    }
    Ok(())
}

fn to_node(item: fetch::Item) -> Option<BookmarkNode> {
    Some(match item {
        fetch::Item::Bookmark { b } => {
            let title = b
                .title
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| b.url.to_string());
            BookmarkNode::Bookmark { url: b.url, title }
        }
        fetch::Item::Folder { f } => BookmarkNode::Folder {
            title: f.title.unwrap_or_default(),
            children: f
                .child_nodes
                .unwrap_or_default()
                .into_iter()
                .filter_map(to_node)
                .collect(),
        },
        fetch::Item::Separator { .. } => BookmarkNode::Separator,
    })
}

fn import_history(
    db: &PlacesDb,
    places: Vec<ImportedPlace>,
    stats: &mut ImportStats,
) -> crate::places::Result<()> {
    const BATCH: usize = 500;
    let mut in_batch = 0;
    let mut tx = Some(db.begin_transaction()?);
    for place in places {
        stats.pages += 1;
        for visit in place.visits {
            let Some(visit_type) = VisitType::from_primitive(visit.visit_type) else {
                continue;
            };
            let observation = VisitObservation::new(place.url.clone())
                .with_visit_type(visit_type)
                .with_at(Timestamp(visit.at_ms))
                .with_title(place.title.clone());
            history::apply_observation_direct(db, observation)?;
            stats.visits += 1;
            in_batch += 1;
            if in_batch >= BATCH {
                if let Some(t) = tx.take() {
                    t.commit()?;
                }
                tx = Some(db.begin_transaction()?);
                in_batch = 0;
            }
        }
    }
    if let Some(t) = tx.take() {
        t.commit()?;
    }
    Ok(())
}

fn import_bookmarks(
    db: &PlacesDb,
    parent: &crate::support::guid::Guid,
    items: Vec<ImportedBookmark>,
) -> crate::places::Result<usize> {
    let mut inserted = 0;
    for item in items {
        match item {
            ImportedBookmark::Bookmark {
                url,
                title,
                added_ms,
            } => {
                if !fetch::fetch_bookmarks_by_url(db, &url)?.is_empty() {
                    continue;
                }
                let bookmark = InsertableBookmark {
                    parent_guid: parent.clone(),
                    position: BookmarkPosition::Append,
                    date_added: Some(Timestamp(added_ms)),
                    last_modified: None,
                    guid: None,
                    url,
                    title,
                };
                bookmarks::insert_bookmark(db, InsertableItem::from(bookmark))?;
                inserted += 1;
            }
            ImportedBookmark::Folder {
                title,
                added_ms,
                children,
            } => {
                let folder = InsertableFolder {
                    parent_guid: parent.clone(),
                    position: BookmarkPosition::Append,
                    date_added: Some(Timestamp(added_ms)),
                    last_modified: None,
                    guid: None,
                    title: Some(title),
                    children: Vec::new(),
                };
                let guid = bookmarks::insert_bookmark(db, InsertableItem::from(folder))?;
                let count = import_bookmarks(db, &guid, children)?;
                if count == 0 {
                    bookmarks::delete_bookmark(db, &guid)?;
                }
                inserted += count;
            }
            ImportedBookmark::Separator => {
                let separator = InsertableSeparator {
                    parent_guid: parent.clone(),
                    position: BookmarkPosition::Append,
                    date_added: None,
                    last_modified: None,
                    guid: None,
                };
                bookmarks::insert_bookmark(db, InsertableItem::from(separator))?;
            }
        }
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("klin-places-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("places.sqlite")
    }

    fn wait<T: Send + 'static>(f: impl FnOnce(Box<dyn FnOnce(T) + Send>)) -> T {
        let (tx, rx) = channel();
        f(Box::new(move |v| tx.send(v).unwrap()));
        rx.recv().unwrap()
    }

    #[test]
    fn visits_titles_search_and_bookmarks() {
        let path = temp_db("svc");
        let places = PlacesService::start(path.clone());
        let servo = Url::parse("https://servo.org/").unwrap();
        let blog = Url::parse("https://servo.org/blog/").unwrap();
        places.record_visit(servo.clone(), Transition::Typed, None);
        places.record_title(servo.clone(), "Servo, the embeddable web engine".into());
        places.record_visit(blog.clone(), Transition::Link, Some(servo.clone()));

        let found = wait(|r| places.search("servo".into(), 10, r));
        assert!(
            found.iter().any(|s| s.url.host_str() == Some("servo.org")),
            "{found:?}"
        );
        let by_title = wait(|r| places.search("embeddable".into(), 10, r));
        assert_eq!(
            by_title.first().map(|s| &s.url),
            Some(&servo),
            "{by_title:?}"
        );

        assert!(!wait(|r| places.is_bookmarked(blog.clone(), r)));
        places.add_bookmark(blog.clone(), Some("Blog".into()));
        places.add_bookmark(blog.clone(), Some("Blog".into()));
        assert!(wait(|r| places.is_bookmarked(blog.clone(), r)));
        places.remove_bookmarks(blog.clone());
        assert!(!wait(|r| places.is_bookmarked(blog.clone(), r)));

        let recent = wait(|r| places.recent_history(10, r));
        assert_eq!(recent.first().map(|e| &e.url), Some(&blog));
        drop(places);

        let places = PlacesService::start(path);
        let recent = wait(|r| places.recent_history(10, r));
        assert_eq!(recent.len(), 2);
        assert_eq!(
            recent[1].title.as_deref(),
            Some("Servo, the embeddable web engine")
        );
    }

    #[test]
    fn import_from_firefox_data() {
        use klin_shared::import::{ImportedBookmark as B, ImportedVisit};
        let places = PlacesService::start(temp_db("import"));
        let url = |s: &str| Url::parse(s).unwrap();
        let history = vec![ImportedPlace {
            url: url("https://www.mozilla.org/"),
            title: Some("Mozilla".into()),
            visits: vec![
                ImportedVisit {
                    at_ms: 1_700_000_000_000,
                    visit_type: 2,
                },
                ImportedVisit {
                    at_ms: 1_700_000_100_000,
                    visit_type: 1,
                },
            ],
        }];
        let roots = || {
            vec![ImportedRoot {
                root: BookmarkRoot::Toolbar,
                children: vec![
                    B::Bookmark {
                        url: url("https://servo.org/"),
                        title: Some("Servo".into()),
                        added_ms: 1,
                    },
                    B::Folder {
                        title: "Rust".into(),
                        added_ms: 1,
                        children: vec![B::Bookmark {
                            url: url("https://www.rust-lang.org/"),
                            title: Some("Rust".into()),
                            added_ms: 1,
                        }],
                    },
                ],
            }]
        };
        let stats = wait(|r| places.import(history.clone(), roots(), r));
        assert_eq!(
            stats,
            ImportStats {
                pages: 1,
                visits: 2,
                bookmarks: 2
            }
        );
        let found = wait(|r| places.search("mozilla".into(), 10, r));
        assert!(
            found
                .iter()
                .any(|s| s.url.host_str() == Some("www.mozilla.org")),
            "{found:?}"
        );
        let recent = wait(|r| places.recent_history(10, r));
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].visited_ms, 1_700_000_100_000);
        assert_eq!(recent[0].title.as_deref(), Some("Mozilla"));
        assert!(wait(
            |r| places.is_bookmarked(url("https://www.rust-lang.org/"), r)
        ));
        let again = wait(|r| places.import(Vec::new(), roots(), r));
        assert_eq!(again.bookmarks, 0);
        let toolbar = wait(|r| places.bookmarks_toolbar(r));
        assert_eq!(toolbar.len(), 2);
        assert!(
            matches!(&toolbar[1], BookmarkNode::Folder { title, children } if title == "Rust" && children.len() == 1)
        );
    }
}
