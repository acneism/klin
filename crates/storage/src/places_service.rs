//! Служба истории и закладок: свой поток с подключением к `places.sqlite`.
//! Поток интерфейса только отправляет сообщения и получает ответы через колбэки.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use url::Url;

use crate::places::api::matcher::{self, SearchParams};
use crate::places::storage::bookmarks::{
    self, BookmarkPosition, BookmarkRootGuid, InsertableBookmark, InsertableItem, fetch,
};
use crate::places::storage::history;
use crate::places::types::{VisitTransitionSet, VisitType};
use crate::places::{ConnectionType, PlacesApi, PlacesDb, VisitObservation};
use crate::support::types::Timestamp;

/// Подсказка адресной строки.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub url: Url,
    pub title: String,
}

/// Запись истории для боковой панели.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: Url,
    pub title: Option<String>,
    /// Время посещения, мс с начала эпохи Unix.
    pub visited_ms: u64,
}

/// Как пользователь попал на страницу — для расчёта frecency (как переходы в Firefox).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Link,
    /// Адрес введён или выбран в адресной строке.
    Typed,
    Bookmark,
    Reload,
}

type Callback<T> = Box<dyn FnOnce(T) + Send>;

enum Message {
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
    /// Открыть (или создать) `places.sqlite` в фоновом потоке.
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

    /// Записать посещение страницы (переход завершился).
    pub fn record_visit(&self, url: Url, transition: Transition, referrer: Option<Url>) {
        self.send(Message::Visit {
            url,
            transition,
            referrer,
        });
    }

    /// Обновить заголовок страницы в истории.
    pub fn record_title(&self, url: Url, title: String) {
        self.send(Message::Title { url, title });
    }

    /// Подсказки для адресной строки по истории и закладкам, лучшие — первыми.
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

    /// Пользователь выбрал подсказку: запомнить пару «ввод → адрес» (адаптивная история Firefox).
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

    /// Удалить все закладки на адрес (как «Удалить закладку» в панели звёздочки Firefox).
    pub fn remove_bookmarks(&self, url: Url) {
        self.send(Message::RemoveBookmarks { url });
    }

    /// Последние посещения, новые — первыми.
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
            // Ответить пустыми данными, чтобы ждущие не зависли.
            for message in rx {
                match message {
                    Message::Search { reply, .. } => reply(Vec::new()),
                    Message::IsBookmarked { reply, .. } => reply(false),
                    Message::RecentHistory { reply, .. } => reply(Vec::new()),
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
            // Наблюдение без типа перехода обновляет страницу, не добавляя посещения.
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
            // search_frecent возвращает результаты отсортированными по адресу; подсказки — по frecency.
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
                // TODO(0.2): сверить папку по умолчанию с `browser.bookmarks.defaultLocation` Firefox.
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

        // Данные на диске: тот же файл открывается заново.
        let places = PlacesService::start(path);
        let recent = wait(|r| places.recent_history(10, r));
        assert_eq!(recent.len(), 2);
        assert_eq!(
            recent[1].title.as_deref(),
            Some("Servo, the embeddable web engine")
        );
    }
}
