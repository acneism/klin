use klin_shared::session::{ClosedTabState, EntryState, Session, TabState, WindowState};
use klin_shared::{LoadStatus, TabId, Url, WindowId};

use crate::browser::{Browser, ClosedTab, Tab, Window, new_tab_url};
use crate::engine::Engine;
use crate::prefs;

impl Browser {
    pub fn session_snapshot(&self, focused: Option<WindowId>) -> Session {
        let windows: Vec<&Window> = self.windows.iter().filter(|w| !w.private).collect();
        let selected_window = focused
            .and_then(|f| windows.iter().position(|w| w.id == f))
            .unwrap_or(0);
        let windows = windows
            .into_iter()
            .map(|w| WindowState {
                tabs: w
                    .tabs
                    .iter()
                    .filter_map(|t| self.tabs.get(t))
                    .map(tab_state)
                    .collect(),
                selected: w.selected,
                closed_tabs: w.closed_tabs.iter().map(closed_tab_state).collect(),
            })
            .collect();
        Session {
            windows,
            selected_window,
        }
    }

    pub fn restore_window(&mut self, engine: &mut dyn Engine, id: WindowId, state: &WindowState) {
        let closed_tabs = state
            .closed_tabs
            .iter()
            .rev()
            .take(prefs::MAX_TABS_UNDO)
            .rev()
            .map(|c| {
                let current = c.tab.current();
                ClosedTab {
                    index: c.pos,
                    url: current.map(|e| e.url.clone()),
                    title: current.map(|e| e.title.clone()).unwrap_or_default(),
                    history: c.tab.entries.iter().map(|e| e.url.clone()).collect(),
                    history_index: c.tab.index,
                    closed_at_ms: c.closed_at_ms,
                }
            })
            .collect();
        self.windows.push(Window {
            id,
            private: false,
            tabs: Vec::new(),
            selected: 0,
            closed_tabs,
        });
        for tab_state in &state.tabs {
            let Some(current) = tab_state.current() else {
                continue;
            };
            let tab = TabId::next();
            self.tabs.insert(tab, lazy_tab(tab, id, tab_state, current));
            if let Some(w) = self.window_mut(id) {
                w.tabs.push(tab);
            }
        }
        let count = self.window(id).map_or(0, |w| w.tabs.len());
        if count == 0 {
            self.open_tab(engine, id, new_tab_url(), None, true);
            return;
        }
        let selected = state.selected.min(count - 1);

        self.select_index(engine, id, selected);
    }
}

fn lazy_tab(id: TabId, window: WindowId, state: &TabState, current: &EntryState) -> Tab {
    let history: Vec<Url> = state.entries.iter().map(|e| e.url.clone()).collect();
    let index = state.index.min(history.len().saturating_sub(1));
    Tab {
        id,
        window,
        url: Some(current.url.clone()),
        title: current.title.clone(),
        favicon: None,
        status_text: None,
        load_status: LoadStatus::Complete,
        can_back: index > 0,
        can_forward: index + 1 < history.len(),
        zoom: 1.0,
        opener: None,
        crashed: None,
        requests: Vec::new(),
        bookmarked: None,
        next_visit: None,
        committed: false,
        history,
        history_index: index,
        loaded: false,
        find_result: None,
        readerable: false,
    }
}

fn tab_state(tab: &Tab) -> TabState {
    let current_url = tab.url.clone().unwrap_or_else(new_tab_url);
    if tab.history.is_empty() {
        return TabState {
            entries: vec![EntryState {
                url: current_url,
                title: tab.title.clone(),
            }],
            index: 0,
        };
    }
    let entries = tab
        .history
        .iter()
        .enumerate()
        .map(|(i, url)| EntryState {
            url: url.clone(),
            title: if i == tab.history_index {
                tab.title.clone()
            } else {
                String::new()
            },
        })
        .collect();
    TabState {
        entries,
        index: tab.history_index,
    }
}

fn closed_tab_state(closed: &ClosedTab) -> ClosedTabState {
    let url = closed.url.clone().unwrap_or_else(new_tab_url);
    let tab = if closed.history.is_empty() {
        TabState {
            entries: vec![EntryState {
                url,
                title: closed.title.clone(),
            }],
            index: 0,
        }
    } else {
        TabState {
            entries: closed
                .history
                .iter()
                .enumerate()
                .map(|(i, u)| EntryState {
                    url: u.clone(),
                    title: if i == closed.history_index {
                        closed.title.clone()
                    } else {
                        String::new()
                    },
                })
                .collect(),
            index: closed.history_index,
        }
    };
    ClosedTabState {
        tab,
        pos: closed.index,
        closed_at_ms: closed.closed_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineEvent;
    use crate::testing::{Call, FakeEngine};

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn snapshot_and_lazy_restore() {
        let mut b = Browser::new();
        let mut e = FakeEngine::default();
        let w = b.open_window(
            &mut e,
            WindowId::next(),
            false,
            Some(url("https://a.example/")),
        );
        let first = b.window(w).unwrap().tabs[0];
        b.handle_event(
            &mut e,
            EngineEvent::TitleChanged {
                tab: first,
                title: "A".into(),
            },
        );
        b.handle_event(
            &mut e,
            EngineEvent::HistoryChanged {
                tab: first,
                entries: vec![url("https://a.example/0"), url("https://a.example/")],
                index: 1,
            },
        );
        b.open_tab(&mut e, w, url("https://b.example/"), None, false);
        b.open_tab(&mut e, w, url("https://c.example/"), None, false);
        let c = b.window(w).unwrap().tabs[2];
        b.close_tab(&mut e, c);

        b.open_window(
            &mut e,
            WindowId::next(),
            true,
            Some(url("https://private.example/")),
        );

        let session = b.session_snapshot(Some(w));
        assert_eq!(session.windows.len(), 1);
        let ws = &session.windows[0];
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.tabs[0].entries.len(), 2);
        assert_eq!(ws.tabs[0].index, 1);
        assert_eq!(ws.tabs[0].current().unwrap().title, "A");
        assert_eq!(
            ws.closed_tabs[0].tab.current().unwrap().url,
            url("https://c.example/")
        );

        let mut b2 = Browser::new();
        let mut e2 = FakeEngine::default();
        let w2 = WindowId::next();
        b2.restore_window(&mut e2, w2, ws);
        let tabs = b2.window(w2).unwrap().tabs.clone();
        assert_eq!(tabs.len(), 2);
        let opened: Vec<_> = e2
            .calls
            .iter()
            .filter(|c| matches!(c, Call::OpenTab(..)))
            .collect();
        assert_eq!(opened.len(), 1);
        assert!(!b2.tab(tabs[1]).unwrap().loaded);
        assert_eq!(b2.tab(tabs[0]).unwrap().title, "A");
        assert!(b2.tab(tabs[0]).unwrap().can_back);

        b2.select_tab(&mut e2, tabs[1]);
        assert!(b2.tab(tabs[1]).unwrap().loaded);
        assert!(e2.calls.contains(&Call::OpenTab(
            tabs[1],
            w2,
            url("https://b.example/"),
            false
        )));

        b2.execute(&mut e2, w2, crate::Command::UndoCloseTab);
        assert_eq!(b2.window(w2).unwrap().tabs.len(), 3);
    }
}
