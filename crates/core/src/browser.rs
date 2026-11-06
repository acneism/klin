//! Модель окон и вкладок и правила её изменения.

use std::collections::HashMap;

use klin_shared::{Disposition, Image, LoadStatus, TabId, Url, WindowId};

use crate::command::Command;
use crate::engine::{Engine, EngineEvent, PageRequest, Reply};
use crate::prefs;
use crate::urifixup::{self, SearchEngine};
use crate::zoom::{self, SiteZoom};

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub window: WindowId,
    pub url: Option<Url>,
    pub title: String,
    /// Значок сайта. `Arc`: интерфейс узнаёт смену значка по указателю, не сравнивая пиксели.
    pub favicon: Option<std::sync::Arc<Image>>,
    pub status_text: Option<String>,
    pub load_status: LoadStatus,
    pub can_back: bool,
    pub can_forward: bool,
    pub zoom: f32,
    /// Вкладка, из которой открыта эта (для `browser.tabs.selectOwnerOnClose`
    /// и `browser.tabs.insertRelatedAfterCurrent`).
    pub opener: Option<TabId>,
    /// Причина сбоя: вкладка показывает страницу сбоя с кнопкой «Перезагрузить».
    pub crashed: Option<String>,
    /// Запросы страницы в порядке поступления (диалоги, разрешения, меню).
    /// Как в Firefox, диалог привязан к вкладке: при переключении вкладок он не теряется.
    pub requests: Vec<PageRequest>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub id: WindowId,
    pub private: bool,
    pub tabs: Vec<TabId>,
    pub selected: usize,
    pub closed_tabs: Vec<ClosedTab>,
}

impl Window {
    pub fn selected_tab(&self) -> Option<TabId> {
        self.tabs.get(self.selected).copied()
    }
}

/// Запись для «Восстановить закрытую вкладку». Firefox восстанавливает и историю сессии
/// вкладки; это придёт вместе с sessionstore в 0.2.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedTab {
    pub index: usize,
    pub url: Option<Url>,
    pub title: String,
}

/// Что интерфейс должен сделать в ответ на команду или событие.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEffect {
    /// Нужно новое окно: интерфейс создаёт окно ОС и вызывает [`Browser::open_window`].
    OpenWindow {
        private: bool,
        url: Option<Url>,
    },
    WindowClosed(WindowId),
    FocusUrlBar(WindowId),
    /// У вкладки появился запрос, который интерфейс не рисует сам (выбор файла).
    NativeRequest {
        tab: TabId,
        request: Box<PageRequest>,
    },
    Redraw(TabId),
    /// Масштаб сайта изменился и должен быть сохранён (`None` — сброшен к 100 %).
    SiteZoomChanged {
        site: String,
        zoom: Option<f32>,
    },
    /// Команда ещё не реализована в этой версии.
    NotImplemented(Command),
}

#[derive(Debug, Default)]
pub struct Browser {
    windows: Vec<Window>,
    tabs: HashMap<TabId, Tab>,
    site_zoom: SiteZoom,
    search: SearchEngine,
}

impl Browser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(&id)
    }

    pub fn selected_tab(&self, window: WindowId) -> Option<&Tab> {
        self.window(window)?
            .selected_tab()
            .and_then(|t| self.tabs.get(&t))
    }

    pub fn search_engine(&self) -> &SearchEngine {
        &self.search
    }

    /// Новое окно с одной вкладкой. Идентификатор выдаёт вызывающий: окно ОС и поверхность
    /// движка для него должны существовать до того, как в окне появится первая вкладка.
    pub fn open_window(
        &mut self,
        engine: &mut dyn Engine,
        id: WindowId,
        private: bool,
        url: Option<Url>,
    ) -> WindowId {
        self.windows.push(Window {
            id,
            private,
            tabs: Vec::new(),
            selected: 0,
            closed_tabs: Vec::new(),
        });
        let url = url.unwrap_or_else(new_tab_url);
        self.open_tab(engine, id, url, None, true);
        id
    }

    /// Открыть вкладку. `opener` — вкладка-источник (ссылка, window.open): такая вкладка
    /// встаёт после текущей и её «родственников», остальные — в конец, как в Firefox.
    pub fn open_tab(
        &mut self,
        engine: &mut dyn Engine,
        window: WindowId,
        url: Url,
        opener: Option<TabId>,
        select: bool,
    ) -> Option<TabId> {
        self.insert_tab(engine, window, url, opener, select, None)
    }

    fn insert_tab(
        &mut self,
        engine: &mut dyn Engine,
        window: WindowId,
        url: Url,
        opener: Option<TabId>,
        select: bool,
        created: Option<TabId>,
    ) -> Option<TabId> {
        let private = self.window(window)?.private;
        let tab = match created {
            Some(tab) => tab,
            None => engine.open_tab(window, &url, private),
        };
        let zoom = host(&url).map_or(1.0, |h| self.site_zoom.get(h));
        if zoom != 1.0 {
            engine.set_zoom(tab, zoom);
        }
        self.tabs.insert(
            tab,
            Tab {
                id: tab,
                window,
                url: Some(url),
                title: String::new(),
                favicon: None,
                status_text: None,
                load_status: LoadStatus::Started,
                can_back: false,
                can_forward: false,
                zoom,
                opener,
                crashed: None,
                requests: Vec::new(),
            },
        );
        let tabs = &self.tabs;
        let win = self.windows.iter_mut().find(|w| w.id == window)?;
        let index = match opener {
            Some(op) if prefs::INSERT_RELATED_AFTER_CURRENT => {
                match win.tabs.iter().position(|&t| t == op) {
                    Some(op_index) => {
                        let mut i = op_index + 1;
                        while i < win.tabs.len()
                            && tabs.get(&win.tabs[i]).and_then(|t| t.opener) == Some(op)
                        {
                            i += 1;
                        }
                        i
                    }
                    None => win.tabs.len(),
                }
            }
            _ => win.tabs.len(),
        };
        win.tabs.insert(index, tab);
        if win.tabs.len() == 1 {
            win.selected = 0;
            engine.set_visible(tab, true);
        } else if select {
            self.select_index(engine, window, index);
        } else {
            let win = self.window_mut(window)?;
            if index <= win.selected {
                win.selected += 1;
            }
            engine.set_visible(tab, false);
        }
        Some(tab)
    }

    fn select_index(&mut self, engine: &mut dyn Engine, window: WindowId, index: usize) {
        let Some(win) = self.window_mut(window) else {
            return;
        };
        if index >= win.tabs.len() {
            return;
        }
        let old = win.selected_tab();
        win.selected = index;
        let new = win.tabs[index];
        if let Some(old) = old.filter(|&o| o != new) {
            engine.set_visible(old, false);
        }
        engine.set_visible(new, true);
    }

    pub fn select_tab(&mut self, engine: &mut dyn Engine, tab: TabId) {
        let Some(window) = self.tabs.get(&tab).map(|t| t.window) else {
            return;
        };
        if let Some(index) = self
            .window(window)
            .and_then(|w| w.tabs.iter().position(|&t| t == tab))
        {
            self.select_index(engine, window, index);
        }
    }

    /// Перетащить вкладку на позицию `index` в её окне. Выбранная вкладка остаётся выбранной.
    pub fn move_tab(&mut self, tab: TabId, index: usize) {
        let Some(window) = self.tabs.get(&tab).map(|t| t.window) else {
            return;
        };
        let Some(win) = self.window_mut(window) else {
            return;
        };
        let Some(from) = win.tabs.iter().position(|&t| t == tab) else {
            return;
        };
        let selected = win.selected_tab();
        let index = index.min(win.tabs.len() - 1);
        let moved = win.tabs.remove(from);
        win.tabs.insert(index, moved);
        if let Some(sel) = selected {
            win.selected = win.tabs.iter().position(|&t| t == sel).unwrap_or(0);
        }
    }

    /// Закрыть вкладку. Возвращает эффекты (например, закрытие окна с последней вкладкой).
    pub fn close_tab(&mut self, engine: &mut dyn Engine, tab: TabId) -> Vec<UiEffect> {
        let Some(closed) = self.tabs.remove(&tab) else {
            return Vec::new();
        };
        for request in &closed.requests {
            engine.answer(request.id, Reply::Dismiss);
        }
        engine.close_tab(tab);
        let Some(win) = self.windows.iter_mut().find(|w| w.id == closed.window) else {
            return Vec::new();
        };
        let Some(index) = win.tabs.iter().position(|&t| t == tab) else {
            return Vec::new();
        };
        let was_selected = index == win.selected;
        win.tabs.remove(index);
        win.closed_tabs.push(ClosedTab {
            index,
            url: closed.url,
            title: closed.title,
        });
        if win.closed_tabs.len() > prefs::MAX_TABS_UNDO {
            win.closed_tabs.remove(0);
        }
        if win.tabs.is_empty() {
            let id = win.id;
            if prefs::CLOSE_WINDOW_WITH_LAST_TAB {
                self.windows.retain(|w| w.id != id);
                return vec![UiEffect::WindowClosed(id)];
            }
            self.open_tab(engine, id, new_tab_url(), None, true);
            return Vec::new();
        }
        if was_selected {
            // `browser.tabs.selectOwnerOnClose`: вернуться к вкладке-источнику, если она в этом окне.
            // Отступление от Firefox: не проверяем, переключался ли пользователь после открытия.
            let owner = closed
                .opener
                .and_then(|o| win.tabs.iter().position(|&t| t == o));
            let next = owner.unwrap_or(index.min(win.tabs.len() - 1));
            win.selected = usize::MAX; // старой вкладки уже нет, скрывать нечего
            let id = win.id;
            self.select_index(engine, id, next);
        } else if index < win.selected {
            win.selected -= 1;
        }
        Vec::new()
    }

    pub fn close_window(&mut self, engine: &mut dyn Engine, window: WindowId) -> Vec<UiEffect> {
        let Some(pos) = self.windows.iter().position(|w| w.id == window) else {
            return Vec::new();
        };
        let win = self.windows.remove(pos);
        for tab in win.tabs {
            self.tabs.remove(&tab);
            engine.close_tab(tab);
        }
        vec![UiEffect::WindowClosed(window)]
    }

    pub fn undo_close_tab(&mut self, engine: &mut dyn Engine, window: WindowId) -> Option<TabId> {
        let closed = self.window_mut(window)?.closed_tabs.pop()?;
        let url = closed.url.unwrap_or_else(new_tab_url);
        let tab = self.open_tab(engine, window, url, None, true)?;
        // Вернуть на прежнее место.
        let win = self.window_mut(window)?;
        let current = win.tabs.iter().position(|&t| t == tab)?;
        let target = closed.index.min(win.tabs.len() - 1);
        let t = win.tabs.remove(current);
        win.tabs.insert(target, t);
        win.selected = target;
        Some(tab)
    }

    /// Ответить на запрос страницы.
    pub fn answer(
        &mut self,
        engine: &mut dyn Engine,
        tab: TabId,
        id: klin_shared::RequestId,
        reply: Reply,
    ) {
        let Some(t) = self.tabs.get_mut(&tab) else {
            return;
        };
        let before = t.requests.len();
        t.requests.retain(|r| r.id != id);
        if t.requests.len() != before {
            engine.answer(id, reply);
        }
    }

    /// Ввод адресной строки: адрес или поиск.
    pub fn load_input(&mut self, engine: &mut dyn Engine, window: WindowId, input: &str) {
        let Some(url) = urifixup::to_url(input, &self.search) else {
            return;
        };
        if let Some(tab) = self.window(window).and_then(Window::selected_tab) {
            engine.load(tab, &url);
        }
    }

    /// Выполнить команду в окне.
    pub fn execute(
        &mut self,
        engine: &mut dyn Engine,
        window: WindowId,
        command: Command,
    ) -> Vec<UiEffect> {
        use Command::*;
        let Some(win) = self.window(window) else {
            return Vec::new();
        };
        let selected = win.selected_tab();
        let count = win.tabs.len();
        let current = win.selected;
        match command {
            NewWindow | NewPrivateWindow => {
                return vec![UiEffect::OpenWindow {
                    private: command == NewPrivateWindow,
                    url: None,
                }];
            }
            CloseWindow => return self.close_window(engine, window),
            NewTab => {
                self.open_tab(engine, window, new_tab_url(), None, true);
                return vec![UiEffect::FocusUrlBar(window)];
            }
            CloseTab => {
                if let Some(tab) = selected {
                    return self.close_tab(engine, tab);
                }
            }
            UndoCloseTab => {
                self.undo_close_tab(engine, window);
            }
            NextTab if count > 0 => self.select_index(engine, window, (current + 1) % count),
            PreviousTab if count > 0 => {
                self.select_index(engine, window, (current + count - 1) % count)
            }
            SelectTab(n) => self.select_index(engine, window, usize::from(n)),
            SelectLastTab if count > 0 => self.select_index(engine, window, count - 1),
            FocusUrlBar => return vec![UiEffect::FocusUrlBar(window)],
            Back | Forward | Reload | ReloadSkipCache | Stop | Home | ZoomIn | ZoomOut
            | ZoomReset => {
                if let Some(tab) = selected {
                    return self.tab_command(engine, tab, command);
                }
            }
            NextTab | PreviousTab | SelectLastTab => {}
            Find
            | FindAgain
            | FindPrevious
            | Bookmark
            | ToggleBookmarksToolbar
            | ShowAllBookmarks
            | ToggleHistorySidebar
            | ShowAllHistory
            | ShowDownloads
            | ToggleReaderMode
            | ToggleFullScreen => return vec![UiEffect::NotImplemented(command)],
        }
        Vec::new()
    }

    fn tab_command(
        &mut self,
        engine: &mut dyn Engine,
        tab: TabId,
        command: Command,
    ) -> Vec<UiEffect> {
        use Command::*;
        match command {
            Back => engine.go_back(tab),
            Forward => engine.go_forward(tab),
            Reload => engine.reload(tab, false),
            ReloadSkipCache => engine.reload(tab, true),
            Stop => engine.stop(tab),
            Home => {
                if let Ok(url) = Url::parse(prefs::HOMEPAGE) {
                    engine.load(tab, &url);
                }
            }
            ZoomIn | ZoomOut | ZoomReset => {
                let Some(t) = self.tabs.get_mut(&tab) else {
                    return Vec::new();
                };
                let new = match command {
                    ZoomIn => zoom::enlarge(t.zoom),
                    ZoomOut => zoom::reduce(t.zoom),
                    _ => 1.0,
                };
                t.zoom = new;
                engine.set_zoom(tab, new);
                let site = t.url.as_ref().and_then(host).map(str::to_owned);
                let window = t.window;
                let private = self.window(window).is_some_and(|w| w.private);
                if let (true, Some(site)) = (prefs::ZOOM_SITE_SPECIFIC, site) {
                    // Как в Firefox: масштаб сайта меняется во всех его вкладках.
                    for other in self.tabs.values_mut() {
                        if other.id != tab
                            && other.url.as_ref().and_then(host) == Some(site.as_str())
                        {
                            other.zoom = new;
                            engine.set_zoom(other.id, new);
                        }
                    }
                    self.site_zoom.set(&site, new);
                    // Масштаб из приватного окна на диск не пишется.
                    if !private {
                        let zoom = (new != 1.0).then_some(new);
                        return vec![UiEffect::SiteZoomChanged { site, zoom }];
                    }
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Загрузить сохранённый масштаб сайтов (из content-prefs) и применить к открытым вкладкам,
    /// где масштаб ещё не меняли.
    pub fn load_site_zoom(
        &mut self,
        engine: &mut dyn Engine,
        values: impl IntoIterator<Item = (String, f32)>,
    ) {
        for (site, zoom) in values {
            self.site_zoom.set(&site, zoom);
            for tab in self.tabs.values_mut() {
                if tab.zoom == 1.0 && tab.url.as_ref().and_then(host) == Some(site.as_str()) {
                    tab.zoom = zoom;
                    engine.set_zoom(tab.id, zoom);
                }
            }
        }
    }

    /// Применить событие движка к модели.
    pub fn handle_event(&mut self, engine: &mut dyn Engine, event: EngineEvent) -> Vec<UiEffect> {
        match event {
            EngineEvent::LoadStatus { tab, status } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.load_status = status;
                    // Новый документ: значок прежней страницы больше не относится к вкладке.
                    if status == LoadStatus::Started {
                        t.favicon = None;
                    }
                }
            }
            EngineEvent::UrlChanged { tab, url } => {
                let Some(t) = self.tabs.get_mut(&tab) else {
                    return Vec::new();
                };
                let host_changed = t.url.as_ref().and_then(host) != host(&url);
                if host_changed && prefs::ZOOM_SITE_SPECIFIC {
                    let z = host(&url).map_or(1.0, |h| self.site_zoom.get(h));
                    if z != t.zoom {
                        t.zoom = z;
                        engine.set_zoom(tab, z);
                    }
                }
                t.url = Some(url);
                t.crashed = None;
            }
            EngineEvent::TitleChanged { tab, title } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.title = title;
                }
            }
            EngineEvent::FaviconChanged { tab, icon } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.favicon = Some(std::sync::Arc::new(icon));
                }
            }
            EngineEvent::StatusTextChanged { tab, text } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.status_text = text;
                }
            }
            EngineEvent::HistoryChanged {
                tab,
                can_back,
                can_forward,
            } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.can_back = can_back;
                    t.can_forward = can_forward;
                }
            }
            EngineEvent::OpenTabRequested {
                opener,
                url,
                disposition,
                created,
            } => {
                let Some(window) = self.tabs.get(&opener).map(|t| t.window) else {
                    return Vec::new();
                };
                match disposition {
                    Disposition::NewWindow => {
                        // Отступление от Firefox (временное): окно из window.open пока открывается
                        // с новой WebView, без связи с opener.
                        if let Some(tab) = created {
                            engine.close_tab(tab);
                        }
                        let private = self.window(window).is_some_and(|w| w.private);
                        return vec![UiEffect::OpenWindow {
                            private,
                            url: Some(url),
                        }];
                    }
                    Disposition::ForegroundTab => {
                        self.insert_tab(engine, window, url, Some(opener), true, created);
                    }
                    Disposition::BackgroundTab => {
                        self.insert_tab(engine, window, url, Some(opener), false, created);
                    }
                }
            }
            EngineEvent::Request { tab, id, kind } => {
                let Some(t) = self.tabs.get_mut(&tab) else {
                    engine.answer(id, Reply::Dismiss);
                    return Vec::new();
                };
                let request = PageRequest { id, kind };
                if matches!(request.kind, crate::RequestKind::FilePicker { .. }) {
                    // Системный диалог выбора файла рисует app; запрос держим во вкладке,
                    // чтобы страница была заблокирована, пока диалог открыт.
                    t.requests.push(request.clone());
                    return vec![UiEffect::NativeRequest {
                        tab,
                        request: Box::new(request),
                    }];
                }
                t.requests.push(request);
                return vec![UiEffect::Redraw(tab)];
            }
            EngineEvent::RequestCancelled { tab, id } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.requests.retain(|r| r.id != id);
                }
                return vec![UiEffect::Redraw(tab)];
            }
            EngineEvent::NewFrame { tab } => return vec![UiEffect::Redraw(tab)],
            EngineEvent::Closed { tab } => {
                // window.close() из страницы.
                if self.tabs.contains_key(&tab) {
                    return self.close_tab(engine, tab);
                }
            }
            EngineEvent::Crashed { tab, reason } => {
                if let Some(t) = self.tabs.get_mut(&tab) {
                    t.crashed = Some(reason);
                    t.load_status = LoadStatus::Complete;
                }
                return vec![UiEffect::Redraw(tab)];
            }
        }
        Vec::new()
    }
}

fn new_tab_url() -> Url {
    Url::parse(prefs::NEW_TAB_URL).expect("valid constant URL")
}

fn host(url: &Url) -> Option<&str> {
    url.host_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Call, FakeEngine};

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn setup() -> (Browser, FakeEngine, WindowId) {
        let mut b = Browser::new();
        let mut e = FakeEngine::default();
        let w = b.open_window(
            &mut e,
            WindowId::next(),
            false,
            Some(url("https://a.example/")),
        );
        (b, e, w)
    }

    fn tab_urls(b: &Browser, w: WindowId) -> Vec<String> {
        b.window(w)
            .unwrap()
            .tabs
            .iter()
            .map(|t| b.tab(*t).unwrap().url.as_ref().unwrap().to_string())
            .collect()
    }

    #[test]
    fn new_tab_goes_to_end_and_is_selected() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::NewTab);
        let win = b.window(w).unwrap();
        assert_eq!(win.tabs.len(), 2);
        assert_eq!(win.selected, 1);
        assert_eq!(tab_urls(&b, w)[1], "about:newtab");
    }

    #[test]
    fn related_tabs_open_after_opener_in_order() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::NewTab);
        b.execute(&mut e, w, Command::SelectTab(0));
        let opener = b.window(w).unwrap().selected_tab().unwrap();
        for u in ["https://r1.example/", "https://r2.example/"] {
            b.handle_event(
                &mut e,
                EngineEvent::OpenTabRequested {
                    opener,
                    url: url(u),
                    disposition: Disposition::BackgroundTab,
                    created: None,
                },
            );
        }
        assert_eq!(
            tab_urls(&b, w),
            [
                "https://a.example/",
                "https://r1.example/",
                "https://r2.example/",
                "about:newtab"
            ]
        );
        assert_eq!(b.window(w).unwrap().selected, 0);
    }

    #[test]
    fn close_and_undo_restores_position() {
        let (mut b, mut e, w) = setup();
        b.open_tab(&mut e, w, url("https://b.example/"), None, false);
        b.open_tab(&mut e, w, url("https://c.example/"), None, false);
        b.execute(&mut e, w, Command::SelectTab(1));
        b.execute(&mut e, w, Command::CloseTab);
        assert_eq!(
            tab_urls(&b, w),
            ["https://a.example/", "https://c.example/"]
        );
        // После закрытия выбирается вкладка справа.
        assert_eq!(
            b.selected_tab(w).unwrap().url.as_ref().unwrap().as_str(),
            "https://c.example/"
        );
        b.execute(&mut e, w, Command::UndoCloseTab);
        assert_eq!(
            tab_urls(&b, w),
            [
                "https://a.example/",
                "https://b.example/",
                "https://c.example/"
            ]
        );
        assert_eq!(b.window(w).unwrap().selected, 1);
    }

    #[test]
    fn closing_owned_tab_returns_to_owner() {
        let (mut b, mut e, w) = setup();
        b.open_tab(&mut e, w, url("https://b.example/"), None, false);
        let owner = b.window(w).unwrap().tabs[0];
        b.handle_event(
            &mut e,
            EngineEvent::OpenTabRequested {
                opener: owner,
                url: url("https://child.example/"),
                disposition: Disposition::ForegroundTab,
                created: None,
            },
        );
        b.execute(&mut e, w, Command::CloseTab);
        assert_eq!(b.window(w).unwrap().selected_tab(), Some(owner));
    }

    #[test]
    fn closing_last_tab_closes_window() {
        let (mut b, mut e, w) = setup();
        let effects = b.execute(&mut e, w, Command::CloseTab);
        assert_eq!(effects, [UiEffect::WindowClosed(w)]);
        assert!(b.windows().is_empty());
    }

    #[test]
    fn new_window_is_requested_from_ui() {
        let (mut b, mut e, w) = setup();
        let effects = b.execute(&mut e, w, Command::NewPrivateWindow);
        assert_eq!(
            effects,
            [UiEffect::OpenWindow {
                private: true,
                url: None
            }]
        );
        let id = WindowId::next();
        b.open_window(&mut e, id, true, None);
        assert!(b.window(id).unwrap().private);
        assert!(
            e.calls
                .iter()
                .any(|c| matches!(c, Call::OpenTab(_, win, _, true) if *win == id))
        );
    }

    #[test]
    fn dragging_a_tab_keeps_selection() {
        let (mut b, mut e, w) = setup();
        b.open_tab(&mut e, w, url("https://b.example/"), None, false);
        b.open_tab(&mut e, w, url("https://c.example/"), None, false);
        let a = b.window(w).unwrap().tabs[0];
        b.move_tab(a, 2);
        assert_eq!(
            tab_urls(&b, w),
            [
                "https://b.example/",
                "https://c.example/",
                "https://a.example/"
            ]
        );
        assert_eq!(b.window(w).unwrap().selected_tab(), Some(a));
        b.move_tab(a, 99);
        assert_eq!(b.window(w).unwrap().tabs[2], a);
    }

    #[test]
    fn ctrl_tab_wraps_around() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::NewTab);
        b.execute(&mut e, w, Command::NextTab);
        assert_eq!(b.window(w).unwrap().selected, 0);
        b.execute(&mut e, w, Command::PreviousTab);
        assert_eq!(b.window(w).unwrap().selected, 1);
    }

    #[test]
    fn only_selected_tab_is_visible() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::NewTab);
        let tabs = b.window(w).unwrap().tabs.clone();
        assert!(!e.visible(tabs[0]));
        assert!(e.visible(tabs[1]));
        b.execute(&mut e, w, Command::SelectTab(0));
        assert!(e.visible(tabs[0]));
        assert!(!e.visible(tabs[1]));
    }

    #[test]
    fn zoom_is_remembered_per_site() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::ZoomIn);
        b.execute(&mut e, w, Command::ZoomIn);
        let first = b.window(w).unwrap().tabs[0];
        assert_eq!(b.tab(first).unwrap().zoom, 1.2);
        // Новая вкладка на том же сайте получает тот же масштаб.
        let second = b
            .open_tab(&mut e, w, url("https://a.example/other"), None, true)
            .unwrap();
        assert_eq!(b.tab(second).unwrap().zoom, 1.2);
        assert!(e.calls.contains(&Call::SetZoom(second, 1.2)));
        // Изменение сохраняется и применяется ко всем вкладкам сайта.
        let effects = b.execute(&mut e, w, Command::ZoomOut);
        assert_eq!(
            effects,
            [UiEffect::SiteZoomChanged {
                site: "a.example".into(),
                zoom: Some(1.1)
            }]
        );
        assert_eq!(b.tab(first).unwrap().zoom, 1.1);
        // Переход на другой сайт сбрасывает масштаб.
        b.handle_event(
            &mut e,
            EngineEvent::UrlChanged {
                tab: second,
                url: url("https://z.example/"),
            },
        );
        assert_eq!(b.tab(second).unwrap().zoom, 1.0);
    }

    #[test]
    fn saved_zoom_applies_to_open_tabs() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        b.load_site_zoom(&mut e, [("a.example".to_owned(), 1.5)]);
        assert_eq!(b.tab(tab).unwrap().zoom, 1.5);
        assert!(e.calls.contains(&Call::SetZoom(tab, 1.5)));
    }

    #[test]
    fn private_window_zoom_is_not_saved() {
        let mut b = Browser::new();
        let mut e = FakeEngine::default();
        let w = b.open_window(
            &mut e,
            WindowId::next(),
            true,
            Some(url("https://p.example/")),
        );
        assert!(b.execute(&mut e, w, Command::ZoomIn).is_empty());
    }

    #[test]
    fn url_bar_input_loads_url_or_search() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        b.load_input(&mut e, w, "example.org");
        b.load_input(&mut e, w, "servo engine");
        assert!(
            e.calls
                .contains(&Call::Load(tab, url("https://example.org/")))
        );
        assert!(e.calls.contains(&Call::Load(
            tab,
            url("https://www.google.com/search?q=servo+engine")
        )));
    }

    #[test]
    fn navigation_commands_reach_engine() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        for c in [
            Command::Back,
            Command::Forward,
            Command::ReloadSkipCache,
            Command::Stop,
        ] {
            b.execute(&mut e, w, c);
        }
        let tail: Vec<_> = e.calls.iter().rev().take(4).rev().cloned().collect();
        assert_eq!(
            tail,
            [
                Call::GoBack(tab),
                Call::GoForward(tab),
                Call::Reload(tab, true),
                Call::Stop(tab)
            ]
        );
    }

    #[test]
    fn requests_belong_to_their_tab() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        let id = klin_shared::RequestId::next();
        let kind = crate::RequestKind::Alert {
            message: "hi".into(),
        };
        b.handle_event(&mut e, EngineEvent::Request { tab, id, kind });
        assert_eq!(b.tab(tab).unwrap().requests.len(), 1);
        b.answer(&mut e, tab, id, Reply::Ok);
        assert!(b.tab(tab).unwrap().requests.is_empty());
        assert_eq!(e.calls.last(), Some(&Call::Answer(id, Reply::Ok)));
        // Повторный ответ движку не уходит.
        let calls = e.calls.len();
        b.answer(&mut e, tab, id, Reply::Ok);
        assert_eq!(e.calls.len(), calls);
    }

    #[test]
    fn closing_tab_dismisses_its_requests() {
        let (mut b, mut e, w) = setup();
        b.execute(&mut e, w, Command::NewTab);
        let tab = b.window(w).unwrap().tabs[1];
        let id = klin_shared::RequestId::next();
        let kind = crate::RequestKind::Confirm {
            message: "?".into(),
        };
        b.handle_event(&mut e, EngineEvent::Request { tab, id, kind });
        b.close_tab(&mut e, tab);
        assert!(e.calls.contains(&Call::Answer(id, Reply::Dismiss)));
    }

    #[test]
    fn favicon_is_reset_when_a_new_page_starts_loading() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        let icon = Image {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        b.handle_event(&mut e, EngineEvent::FaviconChanged { tab, icon });
        assert!(b.tab(tab).unwrap().favicon.is_some());
        b.handle_event(
            &mut e,
            EngineEvent::LoadStatus {
                tab,
                status: LoadStatus::Started,
            },
        );
        assert!(b.tab(tab).unwrap().favicon.is_none());
    }

    #[test]
    fn crash_is_recorded_and_cleared_on_navigation() {
        let (mut b, mut e, w) = setup();
        let tab = b.window(w).unwrap().tabs[0];
        b.handle_event(
            &mut e,
            EngineEvent::Crashed {
                tab,
                reason: "panic".into(),
            },
        );
        assert_eq!(b.tab(tab).unwrap().crashed.as_deref(), Some("panic"));
        b.handle_event(
            &mut e,
            EngineEvent::UrlChanged {
                tab,
                url: url("https://a.example/"),
            },
        );
        assert_eq!(b.tab(tab).unwrap().crashed, None);
    }
}
