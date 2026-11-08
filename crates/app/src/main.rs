mod extension_api;
mod extensions;
mod file_picker;

use std::error::Error;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use klin_core::input::{Key, Modifiers, MouseButton, NamedKey};
use klin_core::prefs::UserPrefs;
use klin_core::{
    Browser, Command, InputEvent, KeyChord, VisitKind, browser::UiEffect, command,
    default_shortcuts,
};
use klin_downloads::{DownloadRequest, DownloadService, default_download_dir};
use klin_engine::{EngineConfig, ServoEngine, Waker};
use klin_profile::{Profile, ProfileError};
use klin_shared::bookmarks::BookmarkNode;

type BookmarkTree = Vec<(klin_shared::import::BookmarkRoot, Vec<BookmarkNode>)>;
use klin_shared::download::DownloadEvent;
use klin_shared::import::ImportedData;
use klin_shared::session::Session;
use klin_shared::{TabId, Url, WindowId};
use klin_storage::ContentPrefs;
use klin_storage::PlacesService;
use klin_storage::content_prefs::FULL_ZOOM;
use klin_storage::places_service::ImportStats;
use klin_storage::places_service::Transition;
use klin_storage::sessionstore::{self, SessionPaths, SessionWriter, StartupSession};
use klin_ui::{Action, BrowserUi};
use log::{info, warn};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{self, KeyCode, ModifiersState, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowAttributes};

const APP_NAME: &str = "Klin";

const SUGGESTIONS_LIMIT: u32 = 10;

const HISTORY_SIDEBAR_LIMIT: usize = 500;

const SESSION_INTERVAL: Duration =
    Duration::from_millis(klin_core::prefs::SESSIONSTORE_INTERVAL_MS);

#[derive(Debug)]
struct Wake;

fn main() -> Result<(), Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| "failed to install rustls crypto provider")?;

    let args = Args::parse(std::env::args().skip(1));
    let profile = match Profile::open(args.profile) {
        Ok(profile) => profile,
        Err(ProfileError::Locked(dir)) => {
            eprintln!(
                "{APP_NAME} уже запущен, но не отвечает. Чтобы открыть новое окно, \
                 закройте старый процесс {APP_NAME}.\nПрофиль: {}",
                dir.display()
            );
            std::process::exit(1);
        }
        Err(e) => return Err(e.into()),
    };
    let start_url = args.url.and_then(|arg| {
        klin_core::urifixup::to_url(&arg, &klin_core::urifixup::SearchEngine::default())
    });

    let prefs = read_user_prefs(&profile.dir);
    let session_paths = SessionPaths::new(&profile.dir);
    let startup = sessionstore::startup_session(
        &session_paths,
        prefs.startup_page(),
        prefs.resume_from_crash(),
    );
    if let StartupSession::Restore {
        after_crash: true, ..
    } = &startup
    {
        info!("restoring the session after a crash");
    }

    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let mut app = App::Starting {
        proxy,
        start_url,
        profile: Some(profile),
        startup: Some(Box::new((startup, session_paths, prefs))),
        migration: args.migration,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[derive(Debug, Default, PartialEq)]
struct Args {
    profile: Option<std::path::PathBuf>,
    url: Option<String>,
    migration: bool,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut out = Args::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-profile" | "--profile" => out.profile = args.next().map(Into::into),
                "-migration" | "--migration" => out.migration = true,
                flag if flag.starts_with('-') && flag.len() > 1 => {
                    warn!("unknown argument {flag}");
                }
                _ if out.url.is_none() => out.url = Some(arg),
                _ => {}
            }
        }
        out
    }
}

enum App {
    Starting {
        proxy: EventLoopProxy<Wake>,
        start_url: Option<Url>,
        profile: Option<Profile>,
        startup: Option<Box<(StartupSession, SessionPaths, UserPrefs)>>,
        migration: bool,
    },
    Running(Box<Running>),
    Failed,
}

struct BrowserWindow {
    id: WindowId,
    private: bool,
    egui_state: egui_winit::State,
    painter: egui_glow::Painter,
    ui: BrowserUi,
    page_rect: [f32; 4],
    popup_rect: Option<[f32; 4]>,
    ui_has_focus: bool,
    page_blocked: bool,
    modifiers: ModifiersState,
    cursor: (f32, f32),
    title: String,
    window: Window,
}

impl BrowserWindow {
    fn popup_point(&self) -> Option<(f32, f32)> {
        let [px, py, w, h] = self.popup_rect?;
        let (lx, ly) = (self.cursor.0 - px, self.cursor.1 - py);
        (lx >= 0.0 && ly >= 0.0 && lx < w && ly < h).then_some((lx, ly))
    }

    fn page_point(&self) -> Option<(f32, f32)> {
        if self.page_blocked || self.popup_point().is_some() {
            return None;
        }
        let [px, py, w, h] = self.page_rect;
        let (lx, ly) = (self.cursor.0 - px, self.cursor.1 - py);
        (lx >= 0.0 && ly >= 0.0 && lx < w && ly < h).then_some((lx, ly))
    }
}

type ImportDone = (
    ImportStats,
    usize,
    klin_storage::LoginsImportStats,
    Option<String>,
);

type AutofillReply = (TabId, String, Vec<klin_shared::logins::SavedLogin>);

type OfferReply = (
    WindowId,
    TabId,
    klin_shared::logins::LoginInput,
    Vec<klin_shared::logins::SavedLogin>,
);

fn login_offer(
    tab: TabId,
    login: klin_shared::logins::LoginInput,
    existing: &[klin_shared::logins::SavedLogin],
) -> Result<klin_ui::LoginOffer, Option<String>> {
    let same_site: Vec<&klin_shared::logins::SavedLogin> = existing
        .iter()
        .filter(|l| l.origin == login.origin)
        .collect();
    let same_user = same_site.iter().find(|l| l.username == login.username);
    if let Some(saved) = same_user {
        if saved.password == login.password {
            return Err(Some(saved.id.clone()));
        }
        return Ok(klin_ui::LoginOffer {
            tab,
            login,
            update: Some(saved.id.clone()),
        });
    }
    if login.username.is_empty() && same_site.len() == 1 {
        let saved = same_site[0];
        if saved.password == login.password {
            return Err(Some(saved.id.clone()));
        }
        let mut login = login;
        login.username = saved.username.clone();
        return Ok(klin_ui::LoginOffer {
            tab,
            login,
            update: Some(saved.id.clone()),
        });
    }
    Ok(klin_ui::LoginOffer {
        tab,
        login,
        update: None,
    })
}

struct Running {
    proxy: EventLoopProxy<Wake>,
    picked_tx: Sender<file_picker::Picked>,
    picked_rx: Receiver<file_picker::Picked>,
    content_prefs: ContentPrefs,
    zoom_rx: Receiver<Vec<(String, f64)>>,
    downloads: DownloadService,
    download_rx: Receiver<DownloadEvent>,
    download_start_tx: Sender<DownloadRequest>,
    download_start_rx: Receiver<DownloadRequest>,
    places: PlacesService,
    suggestions_rx: Receiver<(WindowId, String, Vec<klin_ui::Suggestion>)>,
    suggestions_tx: Sender<(WindowId, String, Vec<klin_ui::Suggestion>)>,
    bookmark_rx: Receiver<(Url, bool)>,
    bookmark_tx: Sender<(Url, bool)>,
    history_rx: Receiver<(WindowId, Vec<klin_ui::HistoryItem>)>,
    history_tx: Sender<(WindowId, Vec<klin_ui::HistoryItem>)>,
    tree_rx: Receiver<(WindowId, BookmarkTree)>,
    tree_tx: Sender<(WindowId, BookmarkTree)>,
    toolbar_rx: Receiver<Vec<BookmarkNode>>,
    toolbar_tx: Sender<Vec<BookmarkNode>>,
    extensions: extensions::Extensions,
    logins: klin_storage::LoginsService,
    logins_tx: Sender<Vec<klin_shared::logins::SavedLogin>>,
    addon_file_tx: Sender<std::path::PathBuf>,
    addon_file_rx: Receiver<std::path::PathBuf>,
    logins_rx: Receiver<Vec<klin_shared::logins::SavedLogin>>,
    autofill_tx: Sender<AutofillReply>,
    autofill_rx: Receiver<AutofillReply>,
    offer_tx: Sender<OfferReply>,
    offer_rx: Receiver<OfferReply>,
    tracking_lists: Option<Arc<klin_content_blocking::Lists>>,
    tracking_rx: Receiver<Arc<klin_content_blocking::Lists>>,
    import_rx: Receiver<Result<ImportedData, String>>,
    import_tx: Sender<Result<ImportedData, String>>,
    import_done_rx: Receiver<ImportDone>,
    import_done_tx: Sender<ImportDone>,
    browser: Browser,
    shortcuts: Vec<(KeyChord, Command)>,
    engine: ServoEngine,
    windows: Vec<BrowserWindow>,
    session_writer: Option<SessionWriter>,
    prefs: UserPrefs,
    last_session: Session,
    saved_session: Session,
    next_session_save: Instant,
    focused: Option<WindowId>,
    _profile: Profile,
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let App::Starting {
            proxy,
            start_url,
            profile,
            startup,
            migration,
        } = self
        else {
            return;
        };
        let (Some(profile), Some(startup)) = (profile.take(), startup.take()) else {
            return;
        };
        match Running::start(
            event_loop,
            proxy.clone(),
            start_url.take(),
            profile,
            *startup,
        ) {
            Ok(running) => {
                if *migration {
                    running.start_firefox_import();
                }
                *self = App::Running(Box::new(running));
            }
            Err(error) => {
                log::error!("failed to start: {error}");
                *self = App::Failed;
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, _: Wake) {
        if let App::Running(app) = self {
            app.spin(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        os_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let App::Running(app) = self {
            if let Some(id) = app
                .windows
                .iter()
                .find(|w| w.window.id() == os_id)
                .map(|w| w.id)
            {
                app.window_event(event_loop, id, event);
            }
            app.spin(event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Running(app) = self {
            app.tick_session(event_loop);
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let App::Running(app) = self {
            app.finish_session();
        }
    }
}

fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let word = match (n % 10, n % 100) {
        (1, r) if r != 11 => one,
        (2..=4, r) if !(12..=14).contains(&r) => few,
        _ => many,
    };
    format!("{n} {word}")
}

fn read_user_prefs(dir: &std::path::Path) -> UserPrefs {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    let (prefs_js, user_js) = (read("prefs.js"), read("user.js"));
    UserPrefs::parse_files(&[&prefs_js, &user_js])
}

impl Running {
    fn start(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Wake>,
        start_url: Option<Url>,
        profile: Profile,
        (startup, session_paths, prefs): (StartupSession, SessionPaths, UserPrefs),
    ) -> Result<Self, Box<dyn Error>> {
        let waker = {
            let proxy = proxy.clone();
            Waker::new(move || {
                let _ = proxy.send_event(Wake);
            })
        };
        let engine = ServoEngine::new(
            waker,
            EngineConfig {
                data_dir: Some(profile.dir.clone()),
                http_cache: Some(profile.cache_dir.join("http-cache")),
            },
        )?;

        let content_prefs = ContentPrefs::start(profile.path("content-prefs.sqlite"));
        let (zoom_tx, zoom_rx) = channel();
        {
            let proxy = proxy.clone();
            content_prefs.get_all(FULL_ZOOM, move |values| {
                let _ = zoom_tx.send(values);
                let _ = proxy.send_event(Wake);
            });
        }
        let (picked_tx, picked_rx) = channel();
        let (download_tx, download_rx) = channel();
        let downloads = {
            let proxy = proxy.clone();
            DownloadService::new(default_download_dir(), download_tx, move || {
                let _ = proxy.send_event(Wake);
            })
        };
        let (download_start_tx, download_start_rx) = channel();
        let places = PlacesService::start(profile.path("places.sqlite"));
        let (suggestions_tx, suggestions_rx) = channel();
        let (bookmark_tx, bookmark_rx) = channel();
        let (history_tx, history_rx) = channel();
        let (import_tx, import_rx) = channel();
        let (toolbar_tx, toolbar_rx) = channel();
        let (tree_tx, tree_rx) = channel();
        let (import_done_tx, import_done_rx) = channel();
        let mut engine = engine;
        let mut extensions = extensions::Extensions::new(
            &profile.dir,
            &prefs.string(extensions::UUIDS_PREF, "{}"),
            &engine,
        );
        extensions.load(&mut engine);
        let logins = klin_storage::LoginsService::start(
            profile.path("logins.db"),
            Arc::new(klin_storage::ProtectedKeyFile::new(
                profile.path("logins.key"),
                klin_platform_win::dpapi::protect,
                klin_platform_win::dpapi::unprotect,
            )),
        );
        if !klin_platform_win::dpapi::PROTECTS {
            log::warn!("logins key is stored unprotected: DPAPI is only available on Windows");
        }
        let (logins_tx, logins_rx) = channel();
        let (addon_file_tx, addon_file_rx) = channel();
        let (autofill_tx, autofill_rx) = channel();
        let (offer_tx, offer_rx) = channel();
        let (tracking_tx, tracking_rx) = channel();
        {
            let proxy = proxy.clone();
            klin_content_blocking::ListUpdater::new(
                profile.path("trackingprotection"),
                engine.user_agent(),
            )
            .spawn(tracking_tx, move || {
                let _ = proxy.send_event(Wake);
            });
        }
        let mut running = Self {
            proxy,
            picked_tx,
            picked_rx,
            content_prefs,
            zoom_rx,
            downloads,
            download_rx,
            download_start_tx,
            download_start_rx,
            places,
            suggestions_rx,
            suggestions_tx,
            bookmark_rx,
            bookmark_tx,
            history_rx,
            history_tx,
            import_rx,
            import_tx,
            toolbar_rx,
            toolbar_tx,
            extensions,
            logins,
            logins_tx,
            logins_rx,
            addon_file_tx,
            addon_file_rx,
            autofill_tx,
            autofill_rx,
            offer_tx,
            offer_rx,
            tracking_lists: None,
            tracking_rx,
            tree_rx,
            tree_tx,
            import_done_rx,
            import_done_tx,
            browser: Browser::new(),
            shortcuts: default_shortcuts(),
            engine,
            windows: Vec::new(),
            session_writer: Some(SessionWriter::start(session_paths)),
            prefs,
            last_session: Session::default(),
            saved_session: Session::default(),
            next_session_save: Instant::now() + SESSION_INTERVAL,
            focused: None,
            _profile: profile,
        };
        running.apply_prefs();
        running.refresh_toolbar();
        match startup {
            StartupSession::Restore { session, .. } => {
                for state in &session.windows {
                    running.restore_window(event_loop, state)?;
                }

                if let (Some(url), Some(first)) = (start_url, running.windows.first().map(|w| w.id))
                {
                    running
                        .browser
                        .open_tab(&mut running.engine, first, url, None, true);
                }
            }
            StartupSession::Fresh => {
                running.open_window(event_loop, false, start_url)?;
            }
        }
        running.extensions.start_backgrounds(&mut running.engine);
        running.persist_extension_uuids();
        running.refresh_addons();
        Ok(running)
    }

    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        private: bool,
        url: Option<Url>,
    ) -> Result<WindowId, Box<dyn Error>> {
        let id = self.create_os_window(event_loop, private, url.is_none())?;
        self.browser.open_window(&mut self.engine, id, private, url);
        self.update_titles();
        Ok(id)
    }

    fn restore_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        state: &klin_shared::session::WindowState,
    ) -> Result<WindowId, Box<dyn Error>> {
        let id = self.create_os_window(event_loop, false, false)?;
        self.browser.restore_window(&mut self.engine, id, state);
        self.update_titles();
        Ok(id)
    }

    fn create_os_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        private: bool,
        focus_url_bar: bool,
    ) -> Result<WindowId, Box<dyn Error>> {
        let window = event_loop.create_window(WindowAttributes::default().with_title(APP_NAME))?;
        let id = WindowId::next();
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        self.engine.add_window(
            id,
            event_loop.display_handle()?,
            window.window_handle()?,
            (size.width, size.height),
            scale,
        )?;
        let gl = self
            .engine
            .gl(id)
            .ok_or("no GL context for the new window")?;
        let painter = egui_glow::Painter::new(gl, "", None, false)
            .map_err(|e| format!("egui painter: {e}"))?;
        let egui_ctx = egui::Context::default();
        klin_ui::configure(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            &window,
            Some(scale),
            window.theme(),
            Some(painter.max_texture_side()),
        );
        let mut ui = BrowserUi::new();
        if focus_url_bar {
            ui.focus_url_bar();
        }
        self.windows.push(BrowserWindow {
            id,
            private,
            egui_state,
            painter,
            ui,
            page_rect: [0.0; 4],
            popup_rect: None,
            ui_has_focus: false,
            page_blocked: false,
            modifiers: ModifiersState::empty(),
            cursor: (0.0, 0.0),
            title: String::new(),
            window,
        });
        Ok(id)
    }

    fn close_window(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let Some(index) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let mut win = self.windows.remove(index);

        self.engine.make_current(id);
        win.painter.destroy();
        self.engine.remove_window(id);
        drop(win);

        if self.windows.is_empty() {
            event_loop.exit();
        }
    }

    fn win(&self, id: WindowId) -> Option<&BrowserWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn win_mut(&mut self, id: WindowId) -> Option<&mut BrowserWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    fn request_redraw(&self, id: WindowId) {
        if let Some(w) = self.win(id) {
            w.window.request_redraw();
        }
    }

    fn spin(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(picked) = self.picked_rx.try_recv() {
            let reply = match picked.files {
                Some(files) => klin_core::Reply::Files(files),
                None => klin_core::Reply::Dismiss,
            };
            self.browser
                .answer(&mut self.engine, picked.tab, picked.id, reply);
            self.redraw_tab(picked.tab);
        }
        while let Ok((window, query, items)) = self.suggestions_rx.try_recv() {
            if let Some(w) = self.win_mut(window) {
                w.ui.set_suggestions(&query, items);
                w.window.request_redraw();
            }
        }
        while let Ok((url, bookmarked)) = self.bookmark_rx.try_recv() {
            self.browser.set_bookmarked(&url, bookmarked);
            for w in &self.windows {
                w.window.request_redraw();
            }
        }
        while let Ok((window, items)) = self.history_rx.try_recv() {
            if let Some(w) = self.win_mut(window) {
                w.ui.set_history(items);
                w.window.request_redraw();
            }
        }
        while let Ok((window, tree)) = self.tree_rx.try_recv() {
            if let Some(w) = self.win_mut(window) {
                w.ui.set_bookmarks_tree(tree);
                w.window.request_redraw();
            }
        }
        while let Ok(path) = self.addon_file_rx.try_recv() {
            self.install_addon(path);
        }
        while let Ok(logins) = self.logins_rx.try_recv() {
            for w in &mut self.windows {
                w.ui.set_logins(logins.clone());
                w.window.request_redraw();
            }
        }
        while let Ok((tab, origin, logins)) = self.autofill_rx.try_recv() {
            let exact: Vec<_> = logins.iter().filter(|l| l.origin == origin).collect();
            if let [login] = exact[..] {
                klin_core::Engine::fill_login(
                    &mut self.engine,
                    tab,
                    &origin,
                    &login.username,
                    &login.password,
                );
            }
        }
        while let Ok((window, tab, login, existing)) = self.offer_rx.try_recv() {
            match login_offer(tab, login, &existing) {
                Ok(offer) => {
                    if let Some(w) = self.win_mut(window) {
                        w.ui.offer_login(offer);
                        w.window.request_redraw();
                    }
                }
                Err(Some(id)) => self.logins.touch(id),
                Err(None) => {}
            }
        }
        let mut lists_changed = false;
        while let Ok(lists) = self.tracking_rx.try_recv() {
            self.tracking_lists = Some(lists);
            lists_changed = true;
        }
        if lists_changed {
            self.apply_tracking_protection();
        }
        while let Ok(items) = self.toolbar_rx.try_recv() {
            for w in &mut self.windows {
                w.ui.set_bookmarks_toolbar(items.clone());
                w.window.request_redraw();
            }
        }
        while let Ok(result) = self.import_rx.try_recv() {
            match result {
                Ok(data) => {
                    let cookies = self.engine.import_cookies(data.cookies);
                    let tx = self.import_done_tx.clone();
                    let proxy = self.proxy.clone();
                    let logins_error = data.logins_error;
                    let logins = data.logins;
                    let import_logins = self.logins.importer();
                    self.places
                        .import(data.places, data.bookmarks, move |stats| {
                            import_logins(
                                logins,
                                Box::new(move |login_stats| {
                                    let _ = tx.send((stats, cookies, login_stats, logins_error));
                                    let _ = proxy.send_event(Wake);
                                }),
                            );
                        });
                }
                Err(error) => self.notify_all(format!("Импорт из Firefox не удался: {error}")),
            }
        }
        while let Ok((stats, cookies, logins, logins_error)) = self.import_done_rx.try_recv() {
            self.refresh_toolbar();
            self.load_logins();
            let mut text = format!(
                "Импортировано из Firefox: {} истории ({}), {}, {}, {}.",
                plural(stats.pages, "страница", "страницы", "страниц"),
                plural(stats.visits, "посещение", "посещения", "посещений"),
                plural(stats.bookmarks, "закладка", "закладки", "закладок"),
                plural(cookies, "cookie", "cookies", "cookies"),
                plural(logins.added, "пароль", "пароля", "паролей"),
            );
            if let Some(error) = logins_error {
                text.push_str(&format!(" Пароли не перенесены: {error}."));
            }
            self.notify_all(text);
        }
        while let Ok(request) = self.download_start_rx.try_recv() {
            self.downloads.start(request);
        }
        let mut downloads_changed = false;
        while let Ok(event) = self.download_rx.try_recv() {
            self.browser.download_event(event);
            downloads_changed = true;
        }
        if downloads_changed {
            for w in &self.windows {
                w.window.request_redraw();
            }
        }
        while let Ok(values) = self.zoom_rx.try_recv() {
            let values = values.into_iter().map(|(site, zoom)| (site, zoom as f32));
            self.browser.load_site_zoom(&mut self.engine, values);
        }
        let events = self.engine.spin();
        if events.is_empty() {
            return;
        }
        for event in events {
            match &event {
                klin_core::EngineEvent::ExtensionMessage { tab, url, message } => {
                    let out = self
                        .extensions
                        .host
                        .handle_console(*tab, url.as_ref(), message);
                    self.extension_outgoing(event_loop, out);
                    continue;
                }
                klin_core::EngineEvent::LoadStatus {
                    tab,
                    status: klin_shared::LoadStatus::Started,
                } => self.extensions.host.navigated(*tab),
                klin_core::EngineEvent::Closed { tab } => {
                    self.extensions.host.navigated(*tab);
                    let windows: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
                    for window in windows {
                        if self.engine.popup_tab(window) == Some(*tab) {
                            self.close_popup(window);
                        }
                    }
                }
                _ => {}
            }
            let tab_update = match &event {
                klin_core::EngineEvent::UrlChanged { tab, .. }
                | klin_core::EngineEvent::TitleChanged { tab, .. } => Some(*tab),
                klin_core::EngineEvent::LoadStatus {
                    tab,
                    status: klin_shared::LoadStatus::Complete,
                } => Some(*tab),
                _ => None,
            };
            let effects = self.browser.handle_event(&mut self.engine, event);
            self.apply(event_loop, effects);
            if let Some(tab) = tab_update {
                self.notify_tab_updated(tab);
            }
        }
        self.update_titles();
        for w in &self.windows {
            w.window.request_redraw();
        }
    }

    fn redraw_tab(&self, tab: klin_shared::TabId) {
        if let Some(t) = self.browser.tab(tab) {
            self.request_redraw(t.window);
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let Some(index) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        self.engine.begin_frame(id);
        let blitter: Option<Arc<klin_engine::PageBlitter>> =
            self.engine.page_blitter(id).map(Arc::new);
        let popup_blitter: Option<Arc<klin_engine::PageBlitter>> =
            self.engine.popup_blitter(id).map(Arc::new);
        let popup_size = self.engine.popup_size(id);
        let w = &mut self.windows[index];
        let ctx = w.egui_state.egui_ctx().clone();
        let raw_input = w.egui_state.take_egui_input(&w.window);
        let mut frame = None;
        let (ui, browser, prefs) = (&mut w.ui, &self.browser, &self.prefs);
        let full = ctx.run_ui(raw_input, |root| {
            let mut paint_page = |painter: &egui::Painter, rect: egui::Rect| {
                let Some(blit) = blitter.clone() else { return };
                let callback = egui_glow::CallbackFn::new(move |info, painter| {
                    let v = info.viewport_in_pixels();
                    blit(
                        painter.gl(),
                        [v.left_px, v.from_bottom_px, v.width_px, v.height_px],
                    );
                });
                painter.add(egui::PaintCallback {
                    rect,
                    callback: Arc::new(callback),
                });
            };
            let ppp = root.ctx().pixels_per_point();
            let mut paint_popup = |painter: &egui::Painter, rect: egui::Rect| {
                let Some(blit) = popup_blitter.clone() else {
                    return;
                };
                let callback = egui_glow::CallbackFn::new(move |info, painter| {
                    let v = info.viewport_in_pixels();
                    blit(
                        painter.gl(),
                        [v.left_px, v.from_bottom_px, v.width_px, v.height_px],
                    );
                });
                painter.add(egui::PaintCallback {
                    rect,
                    callback: Arc::new(callback),
                });
            };
            let popup = popup_size.map(|(pw, ph)| klin_ui::Popup {
                size: egui::vec2(pw as f32 / ppp, ph as f32 / ppp),
                paint: &mut paint_popup,
            });
            frame = Some(ui.show(root, browser, id, prefs, &mut paint_page, popup));
        });
        w.egui_state
            .handle_platform_output(&w.window, full.platform_output);
        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        let size = w.window.inner_size();
        let mut textures = full.textures_delta;
        w.painter.paint_and_update_textures(
            [size.width, size.height],
            full.pixels_per_point,
            &primitives,
            &mut textures,
        );
        self.engine.present(id);

        let w = &mut self.windows[index];
        let repaint_now = full
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero());
        if repaint_now {
            w.window.request_redraw();
        }
        if let Some(frame) = frame {
            w.ui_has_focus = frame.ui_has_focus;

            w.page_blocked = frame.page_blocked;
            let ppp = full.pixels_per_point;
            let r = frame.page_rect;
            w.page_rect = [
                r.min.x * ppp,
                r.min.y * ppp,
                r.width() * ppp,
                r.height() * ppp,
            ];
            w.popup_rect = frame.popup_rect.map(|r| {
                [
                    r.min.x * ppp,
                    r.min.y * ppp,
                    r.width() * ppp,
                    r.height() * ppp,
                ]
            });
            let popup_dismissed = frame.popup_dismissed;
            let (pw, ph) = (w.page_rect[2].round() as u32, w.page_rect[3].round() as u32);
            self.engine.resize_page(id, pw, ph);
            if popup_dismissed {
                self.close_popup(id);
            }
            for action in frame.actions {
                self.act(event_loop, id, action);
            }
        }
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, id: WindowId, action: Action) {
        match action {
            Action::Execute(cmd) => self.execute(event_loop, id, cmd),
            Action::SelectTab(tab) => self.browser.select_tab(&mut self.engine, tab),
            Action::MoveTab { tab, index } => self.browser.move_tab(tab, index),
            Action::Find { tab, request } => self.browser.find(&mut self.engine, tab, request),
            Action::CancelDownload(download) => self.downloads.cancel(download),
            Action::OpenDownload(path) => {
                if let Err(e) = klin_platform_win::shell::open(&path) {
                    warn!("cannot open {}: {e}", path.display());
                }
            }
            Action::ShowDownloadInFolder(path) => {
                if let Err(e) = klin_platform_win::shell::show_in_folder(&path) {
                    warn!("cannot show {}: {e}", path.display());
                }
            }
            Action::ClearDownloads => self.browser.clear_downloads(),
            Action::SetPref(name, value) => {
                self.prefs.set(&name, value);
                self.prefs_changed();
            }
            Action::ResetPref(name) => {
                self.prefs.reset(&name);
                self.prefs_changed();
            }
            Action::ClearHistory => {
                self.places.clear_history();
                let open: Vec<WindowId> = self
                    .windows
                    .iter()
                    .filter(|w| w.ui.history_open())
                    .map(|w| w.id)
                    .collect();
                for window in open {
                    self.load_history(window);
                }
            }
            Action::ImportFromFirefox => self.start_firefox_import(),
            Action::LoadLogins => self.load_logins(),
            Action::PickAddonFile => {
                let tx = self.addon_file_tx.clone();
                let proxy = self.proxy.clone();
                file_picker::pick_addon(move |path| {
                    if let Some(path) = path {
                        let _ = tx.send(path);
                        let _ = proxy.send_event(Wake);
                    }
                });
            }
            Action::InstallAddon(path) => self.install_addon(std::path::PathBuf::from(path)),
            Action::UninstallAddon(addon) => {
                self.extensions.uninstall(&addon, &mut self.engine);
                self.persist_extension_uuids();
                self.refresh_addons();
            }
            Action::SetAddonEnabled { id: addon, enabled } => {
                self.extensions
                    .set_enabled(&addon, enabled, &mut self.engine);
                self.refresh_addons();
            }
            Action::AddonOptions(uuid) => {
                if let Some(url) = self.extensions.options_url(&uuid) {
                    self.browser.open_or_select(&mut self.engine, id, url);
                }
            }
            Action::AddonClicked(uuid) => self.addon_clicked(event_loop, &uuid),
            Action::AddLogin(input) => {
                let this = self.logins_reloader();
                self.logins.save(input, move |result| {
                    if let Err(error) = result {
                        log::error!("save login: {error}");
                    }
                    this();
                });
            }
            Action::UpdateLogin {
                id,
                username,
                password,
            } => {
                let this = self.logins_reloader();
                self.logins.update(id, username, password, move |result| {
                    if let Err(error) = result {
                        log::error!("update login: {error}");
                    }
                    this();
                });
            }
            Action::SaveLogin(offer) => {
                let this = self.logins_reloader();
                match offer.update {
                    Some(id) => self.logins.update(
                        id,
                        offer.login.username,
                        offer.login.password,
                        move |result| {
                            if let Err(error) = result {
                                log::error!("update login: {error}");
                            }
                            this();
                        },
                    ),
                    None => self.logins.save(offer.login, move |result| {
                        if let Err(error) = result {
                            log::error!("save login: {error}");
                        }
                        this();
                    }),
                }
            }
            Action::DeleteLogin(id) => {
                self.logins.delete(id);
                self.load_logins();
            }
            Action::DeleteAllLogins => {
                self.logins.delete_all();
                self.load_logins();
            }
            Action::LoadLibrary => {
                self.load_history(id);
                self.load_tree(id);
            }
            Action::DeleteBookmark(bookmark) => {
                self.places.delete_bookmark(bookmark);
                self.refresh_toolbar();
                self.load_tree(id);
                self.recheck_bookmarks();
            }
            Action::DeletePage(url) => {
                self.places.delete_page(url);
                self.load_history(id);
            }
            Action::OpenInternal(url) => {
                if let Ok(url) = Url::parse(url) {
                    self.browser.open_or_select(&mut self.engine, id, url);
                }
            }
            Action::Suggest(query) => {
                let tx = self.suggestions_tx.clone();
                let proxy = self.proxy.clone();
                let q = query.clone();
                self.places.search(query, SUGGESTIONS_LIMIT, move |found| {
                    let items = found
                        .into_iter()
                        .map(|s| klin_ui::Suggestion {
                            url: s.url,
                            title: s.title,
                        })
                        .collect();
                    let _ = tx.send((id, q, items));
                    let _ = proxy.send_event(Wake);
                });
            }
            Action::NavigateSuggestion { query, url } => {
                self.places.accept_suggestion(query, url.clone());
                self.browser
                    .open_url(&mut self.engine, id, &url, VisitKind::Typed);
            }
            Action::OpenBookmark {
                url,
                new_tab: false,
            } => self
                .browser
                .open_url(&mut self.engine, id, &url, VisitKind::Bookmark),
            Action::OpenBookmark { url, new_tab: true } => {
                self.browser
                    .open_url_in_new_tab(&mut self.engine, id, url, VisitKind::Bookmark)
            }
            Action::OpenUrl(url) => {
                self.browser
                    .open_url(&mut self.engine, id, &url, VisitKind::Link)
            }
            Action::RemoveBookmark(url) => self.remove_bookmark(url),
            Action::ToggleHistorySidebar => self.toggle_history(id),
            Action::CloseTab(tab) => {
                let effects = self.browser.close_tab(&mut self.engine, tab);
                self.apply(event_loop, effects);
            }
            Action::Navigate(input) => self.browser.load_input(&mut self.engine, id, &input),
            Action::FocusPage => self.engine.set_page_focused(id, true),
            Action::Answer {
                tab,
                id: request,
                reply,
            } => self.browser.answer(&mut self.engine, tab, request, reply),
        }
        self.update_titles();
        self.request_redraw(id);
    }

    fn execute(&mut self, event_loop: &ActiveEventLoop, id: WindowId, cmd: Command) {
        let effects = self.browser.execute(&mut self.engine, id, cmd);
        self.apply(event_loop, effects);
        self.request_redraw(id);
    }

    fn apply(&mut self, event_loop: &ActiveEventLoop, effects: Vec<UiEffect>) {
        for effect in effects {
            match effect {
                UiEffect::Redraw(tab) => self.redraw_tab(tab),
                UiEffect::WindowClosed(id) => self.close_window(event_loop, id),
                UiEffect::OpenWindow { private, url } => {
                    if let Err(e) = self.open_window(event_loop, private, url) {
                        log::error!("failed to open a window: {e}");
                    }
                }
                UiEffect::FocusUrlBar(id) => {
                    self.engine.set_page_focused(id, false);
                    if let Some(w) = self.win_mut(id) {
                        w.ui.focus_url_bar();
                        w.window.request_redraw();
                    }
                }
                UiEffect::NativeRequest { tab, request } => {
                    if let klin_core::RequestKind::FilePicker { multiple, accept } =
                        request.kind.clone()
                    {
                        let proxy = self.proxy.clone();
                        file_picker::open(
                            tab,
                            request.id,
                            multiple,
                            accept,
                            self.picked_tx.clone(),
                            move || {
                                let _ = proxy.send_event(Wake);
                            },
                        );
                    }
                }
                UiEffect::StartDownload {
                    window,
                    url,
                    referrer,
                } => {
                    let user_agent = self.engine.user_agent();
                    let tx = self.download_start_tx.clone();
                    let proxy = self.proxy.clone();
                    let request_url = url.clone();
                    self.engine.cookie_header(&url, move |cookie| {
                        let request = DownloadRequest {
                            url: request_url,
                            referrer,
                            cookie,
                            user_agent,
                        };
                        let _ = tx.send(request);
                        let _ = proxy.send_event(Wake);
                    });

                    if let Some(w) = self.win_mut(window) {
                        w.ui.open_downloads();
                        w.window.request_redraw();
                    }
                }
                UiEffect::RecordVisit {
                    url,
                    kind,
                    referrer,
                } => {
                    let transition = match kind {
                        VisitKind::Link => Transition::Link,
                        VisitKind::Typed => Transition::Typed,
                        VisitKind::Bookmark => Transition::Bookmark,
                        VisitKind::Reload => Transition::Reload,
                    };
                    self.places.record_visit(url, transition, referrer);

                    let open: Vec<WindowId> = self
                        .windows
                        .iter()
                        .filter(|w| w.ui.history_open())
                        .map(|w| w.id)
                        .collect();
                    for window in open {
                        self.load_history(window);
                    }
                }
                UiEffect::RecordTitle { url, title } => self.places.record_title(url, title),
                UiEffect::QueryBookmark(url) => {
                    let tx = self.bookmark_tx.clone();
                    let proxy = self.proxy.clone();
                    let u = url.clone();
                    self.places.is_bookmarked(url, move |bookmarked| {
                        let _ = tx.send((u, bookmarked));
                        let _ = proxy.send_event(Wake);
                    });
                }
                UiEffect::AddBookmark { url, title } => {
                    self.places.add_bookmark(url, title);
                    self.refresh_toolbar();
                }
                UiEffect::ShowLibrary { window, bookmarks } => {
                    if let Ok(url) = Url::parse("about:library") {
                        self.browser.open_or_select(&mut self.engine, window, url);
                    }
                    if let Some(w) = self.win_mut(window) {
                        let section = if bookmarks {
                            klin_ui::LibrarySection::Bookmarks(
                                klin_shared::import::BookmarkRoot::Toolbar,
                            )
                        } else {
                            klin_ui::LibrarySection::History
                        };
                        w.ui.show_library(section);
                        w.window.request_redraw();
                    }
                }
                UiEffect::ToggleBookmarksToolbar => {
                    let next = if self.prefs.bookmarks_toolbar() == "always" {
                        "never"
                    } else {
                        "always"
                    };
                    self.prefs.set(
                        klin_core::prefs::BOOKMARKS_TOOLBAR,
                        klin_core::prefs::PrefValue::String(next.into()),
                    );
                    self.prefs_changed();
                    for w in &self.windows {
                        w.window.request_redraw();
                    }
                }
                UiEffect::RemoveBookmark(url) => self.remove_bookmark(url),
                UiEffect::ShowBookmarkPanel(window) => {
                    if let Some(w) = self.win_mut(window) {
                        w.ui.show_bookmark_panel();
                        w.window.request_redraw();
                    }
                }
                UiEffect::ToggleHistorySidebar(window) => self.toggle_history(window),
                UiEffect::Quit => {
                    let ids: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
                    for window in ids {
                        let effects = self.browser.close_window(&mut self.engine, window);
                        self.apply(event_loop, effects);
                    }
                }
                UiEffect::ShowFindBar(window) => {
                    let tab = self.browser.window(window).and_then(|w| w.selected_tab());
                    if let (Some(tab), Some(w)) = (tab, self.win_mut(window)) {
                        w.ui.show_find_bar(tab);
                        w.window.request_redraw();
                    }
                }
                UiEffect::FindAgain { window, backwards } => {
                    let tab = self.browser.window(window).and_then(|w| w.selected_tab());
                    let action = match (tab, self.win_mut(window)) {
                        (Some(tab), Some(w)) => {
                            w.window.request_redraw();
                            w.ui.find_again(tab, backwards)
                        }
                        _ => None,
                    };
                    if let Some(action) = action {
                        self.act(event_loop, window, action);
                    }
                }
                UiEffect::ToggleDownloadsPanel(window) => {
                    if let Some(w) = self.win_mut(window) {
                        w.ui.toggle_downloads();
                        w.window.request_redraw();
                    }
                }
                UiEffect::SiteZoomChanged { site, zoom } => {
                    let zoom = zoom.map(|z| (f64::from(z) * 100.0).round() / 100.0);
                    self.content_prefs.set(&site, FULL_ZOOM, zoom);
                }
                UiEffect::ShowAddons(window) => {
                    if let Ok(url) = Url::parse(klin_ui::ADDONS_URL) {
                        self.browser.open_or_select(&mut self.engine, window, url);
                        self.request_redraw(window);
                    }
                }
                UiEffect::QueryLogins { tab, origin } => {
                    let tx = self.autofill_tx.clone();
                    let proxy = self.proxy.clone();
                    self.logins.for_origin(origin.clone(), move |logins| {
                        let _ = tx.send((tab, origin, logins));
                        let _ = proxy.send_event(Wake);
                    });
                }
                UiEffect::OfferSaveLogin { window, tab, login } => {
                    let tx = self.offer_tx.clone();
                    let proxy = self.proxy.clone();
                    self.logins
                        .for_origin(login.origin.clone(), move |existing| {
                            let _ = tx.send((window, tab, login, existing));
                            let _ = proxy.send_event(Wake);
                        });
                }
                UiEffect::NotImplemented(cmd) => info!("{cmd:?} is planned for a later version"),
            }
        }
    }

    fn tick_session(&mut self, event_loop: &ActiveEventLoop) {
        let snapshot = self.browser.session_snapshot(self.focused);
        if !snapshot.windows.is_empty() {
            self.last_session = snapshot;
        }
        let now = Instant::now();
        if now >= self.next_session_save {
            if self.last_session != self.saved_session
                && let Some(writer) = &self.session_writer
            {
                writer.save(self.last_session.clone());
                self.saved_session = self.last_session.clone();
            }
            self.next_session_save = now + SESSION_INTERVAL;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_session_save));
    }

    fn finish_session(&mut self) {
        if let Some(writer) = self.session_writer.take() {
            writer.finish(self.last_session.clone());
        }
    }

    fn apply_prefs(&mut self) {
        let home = Url::parse(&self.prefs.homepage()).ok();
        self.browser.set_home(home);
        self.browser
            .set_history_enabled(self.prefs.history_enabled());
        let dir = self.prefs.string(klin_core::prefs::DOWNLOAD_DIR, "");
        self.downloads.set_dir(if dir.is_empty() {
            default_download_dir()
        } else {
            dir.into()
        });
        self.apply_tracking_protection();
        self.engine
            .set_pdf_viewer(!self.prefs.known_bool(klin_core::prefs::PDFJS_DISABLED));
        self.browser.set_login_settings(klin_core::LoginSettings {
            remember: self.prefs.known_bool(klin_core::prefs::REMEMBER_SIGNONS),
            autofill: self.prefs.known_bool(klin_core::prefs::AUTOFILL_FORMS),
        });
    }

    fn apply_tracking_protection(&mut self) {
        use klin_content_blocking::{Blocker, Policy, Scope};
        use klin_core::prefs::*;
        let Some(lists) = self.tracking_lists.clone() else {
            return;
        };
        let p = &self.prefs;
        let policy = Policy {
            tracking: Scope::from_prefs(
                p.known_bool(TRACKING_PROTECTION),
                p.known_bool(TRACKING_PROTECTION_PRIVATE),
            ),
            content_list: p.content_blocking_strict(),
            social: p.known_bool(SOCIAL_TRACKING),
            cryptomining: p.known_bool(CRYPTOMINING),
            fingerprinting: p.known_bool(FINGERPRINTING),
            email: Scope::from_prefs(
                p.known_bool(EMAIL_TRACKING),
                p.known_bool(EMAIL_TRACKING_PRIVATE),
            ),
        };
        self.engine.set_blocker(Some(Blocker::new(lists, policy)));
    }

    fn prefs_changed(&mut self) {
        self.apply_prefs();
        let text = self.prefs.to_prefs_js();
        let path = self._profile.path("prefs.js");
        std::thread::spawn(move || {
            let tmp = path.with_extension("js.tmp");
            let result = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path));
            if let Err(e) = result {
                log::error!("cannot write {}: {e}", path.display());
            }
        });
    }

    fn start_firefox_import(&self) {
        let tx = self.import_tx.clone();
        let proxy = self.proxy.clone();
        let temp = std::env::temp_dir().join(format!("klin-firefox-import-{}", std::process::id()));
        std::thread::spawn(move || {
            let result = klin_importer::firefox_base_dir()
                .and_then(|base| klin_importer::default_profile(&base))
                .ok_or_else(|| klin_importer::ImportError::NoProfile.to_string())
                .and_then(|dir| {
                    info!("importing Firefox profile {}", dir.display());
                    klin_importer::import_profile(&dir, &temp).map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
            let _ = proxy.send_event(Wake);
        });
    }

    fn notify_all(&mut self, text: String) {
        for w in &mut self.windows {
            w.ui.set_notice(text.clone());
            w.window.request_redraw();
        }
    }

    fn close_popup(&mut self, window: WindowId) {
        if let Some(tab) = self.engine.close_popup(window) {
            self.extensions.host.navigated(tab);
        }
        if let Some(w) = self.win_mut(window) {
            w.ui.popup_closed();
            w.popup_rect = None;
            w.window.request_redraw();
        }
    }

    fn install_addon(&mut self, path: std::path::PathBuf) {
        match self.extensions.install(&path, &mut self.engine) {
            Ok(name) => self.notify_all(format!("Дополнение «{name}» установлено.")),
            Err(error) => self.notify_all(format!("Не удалось установить дополнение: {error}")),
        }
        self.persist_extension_uuids();
        self.refresh_addons();
    }

    fn logins_reloader(&self) -> impl FnOnce() + Send + 'static {
        let tx = self.logins_tx.clone();
        let proxy = self.proxy.clone();
        let list = self.logins.lister();
        move || {
            list(Box::new(move |logins| {
                let _ = tx.send(logins);
                let _ = proxy.send_event(Wake);
            }));
        }
    }

    fn load_logins(&self) {
        (self.logins_reloader())();
    }

    fn load_tree(&self, window: WindowId) {
        let tx = self.tree_tx.clone();
        let proxy = self.proxy.clone();
        self.places.bookmarks_tree(move |tree| {
            let _ = tx.send((window, tree));
            let _ = proxy.send_event(Wake);
        });
    }

    fn recheck_bookmarks(&self) {
        let mut urls: Vec<Url> = self
            .windows
            .iter()
            .filter_map(|w| self.browser.window(w.id))
            .flat_map(|w| w.tabs.iter())
            .filter_map(|t| self.browser.tab(*t).and_then(|t| t.url.clone()))
            .collect();
        urls.sort();
        urls.dedup();
        for url in urls {
            let tx = self.bookmark_tx.clone();
            let proxy = self.proxy.clone();
            let u = url.clone();
            self.places.is_bookmarked(url, move |bookmarked| {
                let _ = tx.send((u, bookmarked));
                let _ = proxy.send_event(Wake);
            });
        }
    }

    fn refresh_toolbar(&self) {
        let tx = self.toolbar_tx.clone();
        let proxy = self.proxy.clone();
        self.places.bookmarks_toolbar(move |items| {
            let _ = tx.send(items);
            let _ = proxy.send_event(Wake);
        });
    }

    fn remove_bookmark(&mut self, url: Url) {
        self.places.remove_bookmarks(url.clone());
        self.refresh_toolbar();
        self.browser.set_bookmarked(&url, false);
    }

    fn toggle_history(&mut self, window: WindowId) {
        let Some(w) = self.win_mut(window) else {
            return;
        };
        let open = w.ui.toggle_history();
        w.window.request_redraw();
        if open {
            self.load_history(window);
        }
    }

    fn load_history(&self, window: WindowId) {
        let tx = self.history_tx.clone();
        let proxy = self.proxy.clone();
        self.places
            .recent_history(HISTORY_SIDEBAR_LIMIT, move |entries| {
                let items = entries
                    .into_iter()
                    .map(|e| klin_ui::HistoryItem {
                        url: e.url,
                        title: e.title,
                        visited_ms: e.visited_ms,
                    })
                    .collect();
                let _ = tx.send((window, items));
                let _ = proxy.send_event(Wake);
            });
    }

    fn update_titles(&mut self) {
        let browser = &self.browser;
        for w in &mut self.windows {
            let page = browser
                .selected_tab(w.id)
                .map(|t| t.title.trim().to_owned())
                .filter(|t| !t.is_empty());

            let app = if w.private {
                format!("{APP_NAME} — приватный просмотр")
            } else {
                APP_NAME.to_owned()
            };
            let title = match page {
                Some(t) => format!("{t} — {app}"),
                None => app,
            };
            if title != w.title {
                w.window.set_title(&title);
                w.title = title;
            }
        }
    }

    fn send(&mut self, id: WindowId, input: InputEvent) {
        if let Some(tab) = self.browser.window(id).and_then(|w| w.selected_tab()) {
            self.browser.page_input(&mut self.engine, tab, input);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop, id);
            return;
        }
        if let WindowEvent::Focused(true) = event {
            self.focused = Some(id);
        }
        let popup_tab = self.engine.popup_tab(id);
        let Some(w) = self.win_mut(id) else { return };
        let egui_response = w.egui_state.on_window_event(&w.window, &event);
        if egui_response.repaint {
            w.window.request_redraw();
        }
        match event {
            WindowEvent::CloseRequested => {
                let effects = self.browser.close_window(&mut self.engine, id);
                self.apply(event_loop, effects);
            }
            WindowEvent::Resized(size) => {
                self.engine.resize_window(id, size.width, size.height);
                self.request_redraw(id);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.engine.set_scale(id, scale_factor as f32)
            }
            WindowEvent::Focused(focused) => {
                let ui_has_focus = w.ui_has_focus;
                self.engine.set_page_focused(id, focused && !ui_has_focus)
            }
            WindowEvent::ModifiersChanged(m) => w.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let modifiers = modifiers(w.modifiers);
                let ui_has_focus = w.ui_has_focus;
                let pressed = event.state == ElementState::Pressed;
                let key = key(&event.logical_key, event.physical_key, modifiers);
                if pressed {
                    let cmd = key.and_then(|k| {
                        command::lookup(&self.shortcuts, KeyChord::new(modifiers, k))
                    });

                    let ui_owns = ui_has_focus && cmd == Some(Command::Stop);
                    if let Some(cmd) = cmd.filter(|_| !ui_owns) {
                        self.execute(event_loop, id, cmd);
                        return;
                    }
                }
                let popup = popup_tab;
                if ui_has_focus && popup.is_none() {
                    return;
                }
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => Some(format!("{code:?}")),
                    PhysicalKey::Unidentified(_) => None,
                };
                let input = InputEvent::Key {
                    key: key.unwrap_or(Key::Named(NamedKey::Other)),
                    text: event.text.map(|t| t.to_string()),
                    code,
                    modifiers,
                    pressed,
                    repeat: event.repeat,
                };
                match popup {
                    Some(popup) => klin_core::Engine::send_input(&mut self.engine, popup, input),
                    None => self.send(id, input),
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                w.cursor = (position.x as f32, position.y as f32);
                if let Some((x, y)) = w.popup_point()
                    && let Some(popup) = popup_tab
                {
                    klin_core::Engine::send_input(
                        &mut self.engine,
                        popup,
                        InputEvent::MouseMove { x, y },
                    );
                } else if let Some((x, y)) = w.page_point() {
                    self.send(id, InputEvent::MouseMove { x, y });
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Back => MouseButton::Back,
                    winit::event::MouseButton::Forward => MouseButton::Forward,
                    winit::event::MouseButton::Other(_) => return,
                };
                let pressed = state == ElementState::Pressed;

                if pressed && matches!(button, MouseButton::Back | MouseButton::Forward) {
                    let cmd = if button == MouseButton::Back {
                        Command::Back
                    } else {
                        Command::Forward
                    };
                    self.execute(event_loop, id, cmd);
                    return;
                }
                if let Some((x, y)) = w.popup_point()
                    && let Some(popup) = popup_tab
                {
                    klin_core::Engine::send_input(
                        &mut self.engine,
                        popup,
                        InputEvent::MouseButton {
                            button,
                            x,
                            y,
                            pressed,
                        },
                    );
                } else if let Some((x, y)) = w.page_point() {
                    self.send(
                        id,
                        InputEvent::MouseButton {
                            button,
                            x,
                            y,
                            pressed,
                        },
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 76.0, f64::from(y) * 76.0),
                    MouseScrollDelta::PixelDelta(d) => (d.x, d.y),
                };
                if let Some((x, y)) = w.popup_point()
                    && let Some(popup) = popup_tab
                {
                    klin_core::Engine::send_input(
                        &mut self.engine,
                        popup,
                        InputEvent::Wheel { dx, dy, x, y },
                    );
                } else if let Some((x, y)) = w.page_point() {
                    self.send(id, InputEvent::Wheel { dx, dy, x, y });
                }
            }
            _ => {}
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        for w in &mut self.windows {
            self.engine.make_current(w.id);
            w.painter.destroy();
        }
    }
}

fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        ctrl: state.control_key(),
        shift: state.shift_key(),
        alt: state.alt_key(),
        meta: state.super_key(),
    }
}

fn key(logical: &keyboard::Key, physical: PhysicalKey, mods: Modifiers) -> Option<Key> {
    use keyboard::NamedKey as W;
    if (mods.ctrl || mods.alt)
        && let PhysicalKey::Code(code) = physical
        && let Some(c) = latin_from_code(code)
    {
        return Some(Key::Char(c));
    }
    Some(match logical {
        keyboard::Key::Character(s) => Key::Char(s.chars().next()?.to_ascii_lowercase()),
        keyboard::Key::Named(named) => Key::Named(match named {
            W::Enter => NamedKey::Enter,
            W::Escape => NamedKey::Escape,
            W::Tab => NamedKey::Tab,
            W::Backspace => NamedKey::Backspace,
            W::Delete => NamedKey::Delete,
            W::Home => NamedKey::Home,
            W::End => NamedKey::End,
            W::PageUp => NamedKey::PageUp,
            W::PageDown => NamedKey::PageDown,
            W::ArrowLeft => NamedKey::ArrowLeft,
            W::ArrowRight => NamedKey::ArrowRight,
            W::ArrowUp => NamedKey::ArrowUp,
            W::ArrowDown => NamedKey::ArrowDown,
            W::F1 => NamedKey::F(1),
            W::F2 => NamedKey::F(2),
            W::F3 => NamedKey::F(3),
            W::F4 => NamedKey::F(4),
            W::F5 => NamedKey::F(5),
            W::F6 => NamedKey::F(6),
            W::F7 => NamedKey::F(7),
            W::F8 => NamedKey::F(8),
            W::F9 => NamedKey::F(9),
            W::F10 => NamedKey::F(10),
            W::F11 => NamedKey::F(11),
            W::F12 => NamedKey::F(12),
            W::Space => return Some(Key::Char(' ')),
            _ => NamedKey::Other,
        }),
        _ => return None,
    })
}

fn latin_from_code(code: KeyCode) -> Option<char> {
    let name = format!("{code:?}");
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.chars().next().map(|c| c.to_ascii_lowercase());
    }
    if let Some(digit) = name
        .strip_prefix("Digit")
        .or_else(|| name.strip_prefix("Numpad"))
        && digit.len() == 1
    {
        return digit.chars().next();
    }
    match code {
        KeyCode::Equal => Some('='),
        KeyCode::Minus | KeyCode::NumpadSubtract => Some('-'),
        KeyCode::NumpadAdd => Some('+'),
        _ => None,
    }
}

#[cfg(test)]
mod login_offer_tests {
    use super::*;
    use klin_shared::logins::{LoginInput, SavedLogin};

    fn saved(id: &str, username: &str, password: &str) -> SavedLogin {
        SavedLogin {
            id: id.into(),
            origin: "https://a.example".into(),
            username: username.into(),
            password: password.into(),
            ..Default::default()
        }
    }

    fn input(username: &str, password: &str) -> LoginInput {
        LoginInput {
            origin: "https://a.example".into(),
            username: username.into(),
            password: password.into(),
            ..Default::default()
        }
    }

    #[test]
    fn decides_between_save_update_and_nothing() {
        let tab = TabId::next();
        let existing = vec![saved("1", "ann", "old")];
        assert_eq!(
            login_offer(tab, input("ann", "old"), &existing),
            Err(Some("1".into()))
        );
        let update = login_offer(tab, input("ann", "new"), &existing).unwrap();
        assert_eq!(update.update.as_deref(), Some("1"));
        let new = login_offer(tab, input("bob", "pw"), &existing).unwrap();
        assert_eq!(new.update, None);
        let no_name = login_offer(tab, input("", "newer"), &existing).unwrap();
        assert_eq!(no_name.login.username, "ann");
        assert_eq!(no_name.update.as_deref(), Some("1"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn russian_plurals() {
        assert_eq!(plural(1, "закладка", "закладки", "закладок"), "1 закладка");
        assert_eq!(plural(3, "закладка", "закладки", "закладок"), "3 закладки");
        assert_eq!(
            plural(12, "закладка", "закладки", "закладок"),
            "12 закладок"
        );
        assert_eq!(
            plural(21, "закладка", "закладки", "закладок"),
            "21 закладка"
        );
    }

    #[test]
    fn firefox_style_arguments() {
        assert_eq!(
            parse(&["-profile", "C:/p", "example.com"]),
            Args {
                profile: Some("C:/p".into()),
                url: Some("example.com".into()),
                migration: false,
            }
        );
        assert_eq!(parse(&["--profile", "x"]).profile, Some("x".into()));
        assert_eq!(
            parse(&["https://servo.org"]).url.as_deref(),
            Some("https://servo.org")
        );
        assert_eq!(parse(&[]), Args::default());
    }
}
