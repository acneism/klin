mod bookmarks_bar;
mod downloads;
mod favicons;
mod findbar;
mod icons;
mod library_ui;
mod logins_ui;
mod places_ui;
mod protections_ui;
mod requests;
mod settings_ui;
mod url_display;

use egui::{Color32, CornerRadius, Key, Rect, RichText, Sense, Vec2};
use icons::Icon;
use klin_core::browser::Tab;
use klin_core::{Browser, Command};
use klin_shared::{LoadStatus, TabId, WindowId};

pub use library_ui::LibrarySection;
pub use logins_ui::LOGINS_URL;
pub use places_ui::{HistoryItem, Suggestion};
pub use url_display::display_url;

pub fn configure(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.visuals.popup_shadow = egui::Shadow::NONE;
        style.visuals.window_shadow = egui::Shadow::NONE;
    });
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Execute(Command),
    SelectTab(TabId),
    CloseTab(TabId),
    Navigate(String),
    FocusPage,
    CancelDownload(klin_shared::DownloadId),
    OpenDownload(std::path::PathBuf),
    ShowDownloadInFolder(std::path::PathBuf),
    ClearDownloads,
    Suggest(String),
    NavigateSuggestion {
        query: String,
        url: klin_shared::Url,
    },
    OpenUrl(klin_shared::Url),
    RemoveBookmark(klin_shared::Url),
    ToggleHistorySidebar,
    Find {
        tab: TabId,
        request: klin_core::FindRequest,
    },
    SetPref(String, klin_core::prefs::PrefValue),
    LoadLibrary,
    LoadLogins,
    AddLogin(klin_shared::logins::LoginInput),
    UpdateLogin {
        id: String,
        username: String,
        password: String,
    },
    DeleteLogin(String),
    DeleteAllLogins,
    DeleteBookmark(String),
    DeletePage(klin_shared::Url),
    OpenBookmark {
        url: klin_shared::Url,
        new_tab: bool,
    },
    ResetPref(String),
    ClearHistory,
    ImportFromFirefox,
    OpenInternal(&'static str),
    MoveTab {
        tab: TabId,
        index: usize,
    },
    Answer {
        tab: TabId,
        id: klin_shared::RequestId,
        reply: klin_core::Reply,
    },
}

pub struct Frame {
    pub actions: Vec<Action>,
    pub page_rect: Rect,
    pub ui_has_focus: bool,
    pub page_blocked: bool,
}

#[derive(Default)]
pub struct BrowserUi {
    url_text: String,
    editing: bool,
    focus_url_bar: bool,
    shown_tab: Option<TabId>,
    requests: requests::RequestUi,
    favicons: favicons::Favicons,
    downloads_open: bool,
    downloads_button: Option<Rect>,
    suggestions: Vec<Suggestion>,
    suggestions_query: String,
    suggestion_selected: Option<usize>,
    url_bar_rect: Option<Rect>,
    star_button: Option<Rect>,
    bookmark_panel_open: bool,
    shield_button: Option<Rect>,
    protections_open: bool,
    history_open: bool,
    history: Vec<HistoryItem>,
    history_filter: String,
    find_bars: findbar::FindBars,
    notice: Option<String>,
    settings: settings_ui::SettingsState,
    bookmarks_toolbar: Vec<klin_shared::bookmarks::BookmarkNode>,
    library: library_ui::LibraryState,
    logins: logins_ui::LoginsState,
    bookmarks_tree: Vec<(
        klin_shared::import::BookmarkRoot,
        Vec<klin_shared::bookmarks::BookmarkNode>,
    )>,
}

fn url_bar_id() -> egui::Id {
    egui::Id::new("klin-url-bar")
}
const TAB_MIN_WIDTH: f32 = 76.0;
const TAB_MAX_WIDTH: f32 = 225.0;

impl BrowserUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focus_url_bar(&mut self) {
        self.focus_url_bar = true;
    }

    pub fn set_suggestions(&mut self, query: &str, items: Vec<Suggestion>) {
        if self.editing && self.url_text.trim() == query {
            self.suggestions = items;
            self.suggestions_query = query.to_owned();
            self.suggestion_selected = None;
        }
    }

    pub fn set_logins(&mut self, logins: Vec<klin_shared::logins::SavedLogin>) {
        self.logins.set_logins(logins);
    }

    pub fn show_library(&mut self, section: LibrarySection) {
        self.library.section = section;
        self.library.requested = false;
    }

    pub fn set_bookmarks_tree(
        &mut self,
        tree: Vec<(
            klin_shared::import::BookmarkRoot,
            Vec<klin_shared::bookmarks::BookmarkNode>,
        )>,
    ) {
        self.bookmarks_tree = tree;
    }

    pub fn set_bookmarks_toolbar(&mut self, items: Vec<klin_shared::bookmarks::BookmarkNode>) {
        self.bookmarks_toolbar = items;
    }

    pub fn set_notice(&mut self, text: String) {
        self.notice = Some(text);
    }

    pub fn show_find_bar(&mut self, tab: TabId) {
        self.find_bars.open(tab);
    }

    pub fn find_again(&mut self, tab: TabId, backwards: bool) -> Option<Action> {
        self.find_bars.again(tab, backwards)
    }

    pub fn show_bookmark_panel(&mut self) {
        self.bookmark_panel_open = true;
    }

    pub fn toggle_history(&mut self) -> bool {
        self.history_open = !self.history_open;
        self.history_open
    }

    pub fn history_open(&self) -> bool {
        self.history_open
    }

    pub fn set_history(&mut self, items: Vec<HistoryItem>) {
        self.history = items;
    }

    pub fn toggle_downloads(&mut self) {
        self.downloads_open = !self.downloads_open;
    }

    pub fn open_downloads(&mut self) {
        self.downloads_open = true;
    }

    pub fn show(
        &mut self,
        root: &mut egui::Ui,
        browser: &Browser,
        window: WindowId,
        prefs: &klin_core::prefs::UserPrefs,
        paint_page: &mut dyn FnMut(&egui::Painter, Rect),
    ) -> Frame {
        let mut actions = Vec::new();
        let selected = browser.selected_tab(window);
        self.sync_url(selected);

        let ctx = root.ctx().clone();
        egui::Panel::top("klin-tabs").show(root, |ui| {
            self.tab_strip(ui, browser, window, &mut actions);
        });
        egui::Panel::top("klin-nav").show(root, |ui| {
            self.nav_bar(ui, selected, browser.downloads(), &mut actions);
        });
        let toolbar_setting = prefs.bookmarks_toolbar();
        if bookmarks_bar::visible(&toolbar_setting, selected.and_then(|t| t.url.as_ref())) {
            egui::Panel::top("klin-bookmarks-bar").show(root, |ui| {
                bookmarks_bar::show(ui, &self.bookmarks_toolbar, &mut actions);
            });
        }

        if let Some(text) = self.notice.clone() {
            egui::Panel::top("klin-notice").show(root, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, true, Icon::Close, 22.0).clicked() {
                            self.notice = None;
                        }
                    });
                });
                ui.add_space(2.0);
            });
        }
        if browser.window(window).is_some_and(|w| w.private) {
            egui::Panel::top("klin-private-notice").show(root, |ui| {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Приватный просмотр").strong());
                    ui.label(
                        "История посещений и сессия этого окна не сохраняются. Но cookies и данные \
                         сайтов пока общие с обычными окнами: движок Servo ещё не умеет их разделять.",
                    );
                });
                ui.add_space(2.0);
            });
        }
        if self.history_open {
            egui::Panel::left("klin-history-sidebar")
                .resizable(true)
                .default_size(300.0)
                .show(root, |ui| {
                    places_ui::history_sidebar(
                        ui,
                        &self.history,
                        &mut self.history_filter,
                        &mut actions,
                    );
                });
        }

        if let Some(tab) = selected
            && let Some(bar) = self.find_bars.get_mut(tab.id)
        {
            egui::Panel::bottom("klin-findbar").show(root, |ui| {
                findbar::show(ui, tab, bar, &mut actions);
            });
        }
        if let Some(win) = browser.window(window) {
            self.find_bars.retain(|t| win.tabs.contains(&t));
        }

        let mut page_rect = Rect::NOTHING;
        let mut native_page = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                page_rect = ui.max_rect();
                let internal = selected
                    .and_then(|t| t.url.as_ref())
                    .map(|u| u.as_str())
                    .filter(|u| {
                        *u == settings_ui::PREFERENCES_URL
                            || *u == settings_ui::CONFIG_URL
                            || *u == library_ui::LIBRARY_URL
                            || *u == logins_ui::LOGINS_URL
                    });
                if internal != Some(logins_ui::LOGINS_URL) {
                    self.logins.requested = false;
                }
                if let Some(url) = internal {
                    native_page = true;
                    ui.painter()
                        .rect_filled(page_rect, 0.0, ui.visuals().panel_fill);
                    if url == logins_ui::LOGINS_URL {
                        logins_ui::show(ui, &mut self.logins, &mut actions);
                    } else if url == library_ui::LIBRARY_URL {
                        let data = library_ui::LibraryData {
                            history: &self.history,
                            bookmarks: &self.bookmarks_tree,
                            downloads: browser.downloads(),
                        };
                        library_ui::show(ui, &mut self.library, data, &mut actions);
                    } else if url == settings_ui::PREFERENCES_URL {
                        settings_ui::preferences_page(ui, prefs, &mut self.settings, &mut actions);
                    } else {
                        settings_ui::config_page(ui, prefs, &mut self.settings, &mut actions);
                    }
                    return;
                }
                match selected.and_then(|t| t.crashed.as_ref().map(|r| (t, r))) {
                    Some((_, reason)) => crash_page(ui, reason, &mut actions),
                    None => {
                        let response = ui.allocate_rect(page_rect, Sense::click());
                        paint_page(ui.painter(), page_rect);
                        if response.clicked() || response.secondary_clicked() {
                            actions.push(Action::FocusPage);
                        }
                    }
                }
                if let Some(status) = selected.and_then(|t| t.status_text.as_deref()) {
                    status_panel(ui, page_rect, status);
                }
            });

        if let (Some(anchor), true, Some(tab)) =
            (self.star_button, self.bookmark_panel_open, selected)
        {
            self.bookmark_panel_open = match &tab.url {
                Some(url) => {
                    places_ui::bookmark_panel(&ctx, anchor, url, tab.title.trim(), &mut actions)
                }
                None => false,
            };
        }
        if actions
            .iter()
            .any(|a| matches!(a, Action::RemoveBookmark(_)))
        {
            self.bookmark_panel_open = false;
        }

        if let (Some(anchor), true, Some(tab)) =
            (self.shield_button, self.protections_open, selected)
        {
            self.protections_open = protections_ui::panel(&ctx, anchor, tab, &mut actions);
        } else {
            self.protections_open = false;
        }

        if let (Some(anchor), true) = (self.downloads_button, self.downloads_open) {
            self.downloads_open = downloads::panel(&ctx, anchor, browser.downloads(), &mut actions);
        }

        let page_blocked = native_page
            || match selected {
                Some(tab) => self.requests.show(&ctx, tab, page_rect, &mut actions),
                None => false,
            };
        if page_blocked {
            actions.retain(|a| *a != Action::FocusPage);
        }
        if actions.contains(&Action::FocusPage) {
            ctx.memory_mut(|m| m.surrender_focus(url_bar_id()));
        }
        let ui_has_focus = page_blocked || ctx.memory(|m| m.focused().is_some());
        Frame {
            actions,
            page_rect,
            ui_has_focus,
            page_blocked,
        }
    }

    fn sync_url(&mut self, tab: Option<&Tab>) {
        let id = tab.map(|t| t.id);
        if id != self.shown_tab {
            self.shown_tab = id;
            self.editing = false;
        }
        if !self.editing {
            self.url_text = tab
                .and_then(|t| t.url.as_ref())
                .map(display_url)
                .unwrap_or_default();
        }
    }

    fn tab_strip(
        &mut self,
        ui: &mut egui::Ui,
        browser: &Browser,
        window: WindowId,
        actions: &mut Vec<Action>,
    ) {
        let Some(win) = browser.window(window) else {
            return;
        };
        self.favicons.retain(|tab| win.tabs.contains(&tab));
        if win
            .tabs
            .iter()
            .filter_map(|t| browser.tab(*t))
            .any(|t| t.load_status != LoadStatus::Complete)
        {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let count = win.tabs.len().max(1) as f32;
            let available = ui.available_width() - 40.0;
            let width = (available / count).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
            egui::ScrollArea::horizontal()
                .max_width(available)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut rects = Vec::with_capacity(win.tabs.len());
                        let mut dragged = None;
                        for (index, &tab_id) in win.tabs.iter().enumerate() {
                            let Some(tab) = browser.tab(tab_id) else {
                                continue;
                            };
                            let icon = self.favicons.texture(ui.ctx(), tab);
                            let (rect, is_dragged) =
                                tab_button(ui, tab, icon, index == win.selected, width, actions);
                            if is_dragged {
                                dragged = Some((tab_id, index));
                            }
                            rects.push((tab_id, rect));
                        }

                        if let (Some((tab, from)), Some(pointer)) =
                            (dragged, ui.ctx().pointer_interact_pos())
                        {
                            let to = rects
                                .iter()
                                .filter(|(id, r)| *id != tab && r.center().x < pointer.x)
                                .count();
                            if to != from {
                                actions.push(Action::MoveTab { tab, index: to });
                            }
                        }
                    });
                });
            if icons::button(ui, true, Icon::Plus, 28.0)
                .on_hover_text("Открыть новую вкладку (Ctrl+T)")
                .clicked()
            {
                actions.push(Action::Execute(Command::NewTab));
            }
        });
        ui.add_space(2.0);
    }

    fn nav_bar(
        &mut self,
        ui: &mut egui::Ui,
        tab: Option<&Tab>,
        downloads: &[klin_shared::download::Download],
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            let (can_back, can_forward) =
                tab.map_or((false, false), |t| (t.can_back, t.can_forward));
            let loading = tab.is_some_and(|t| t.load_status != LoadStatus::Complete);
            let mut nav =
                |ui: &mut egui::Ui, enabled: bool, icon: Icon, hint: &str, cmd: Command| {
                    if icons::button(ui, enabled, icon, 30.0)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        actions.push(Action::Execute(cmd));
                    }
                };
            nav(
                ui,
                can_back,
                Icon::Back,
                "Назад на одну страницу (Alt+←)",
                Command::Back,
            );
            nav(
                ui,
                can_forward,
                Icon::Forward,
                "Вперёд на одну страницу (Alt+→)",
                Command::Forward,
            );
            if loading {
                nav(
                    ui,
                    true,
                    Icon::Stop,
                    "Остановить загрузку этой страницы (Esc)",
                    Command::Stop,
                );
            } else {
                nav(
                    ui,
                    tab.is_some(),
                    Icon::Reload,
                    "Обновить текущую страницу (F5)",
                    Command::Reload,
                );
            }
            nav(
                ui,
                true,
                Icon::Home,
                "Домашняя страница (Alt+Home)",
                Command::Home,
            );
            self.shield_button = None;
            if let Some(shield) = protections_ui::shield_button(ui, tab) {
                if shield.clicked() {
                    self.protections_open = !self.protections_open;
                }
                self.shield_button = Some(shield.rect);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let zoom = tab.map(|t| t.zoom);
                settings_ui::menu_button(ui, &mut self.settings, zoom, actions);
                self.downloads_button = None;
                if let Some(button) = downloads::button(ui, downloads) {
                    if button.clicked() {
                        self.downloads_open = !self.downloads_open;
                    }
                    self.downloads_button = Some(button.rect);
                }
                let can_bookmark = tab
                    .and_then(|t| t.url.as_ref())
                    .is_some_and(|u| matches!(u.scheme(), "http" | "https" | "file"));
                let bookmarked = tab.and_then(|t| t.bookmarked) == Some(true);
                let reader_active = tab
                    .and_then(|t| t.url.as_ref())
                    .is_some_and(klin_core::reader::is_reader);
                if reader_active || tab.is_some_and(|t| t.readerable) {
                    let icon = if reader_active {
                        Icon::ReaderActive
                    } else {
                        Icon::Reader
                    };
                    let hint = if reader_active {
                        "Закрыть режим чтения (F9)"
                    } else {
                        "Перейти в режим чтения (F9)"
                    };
                    if icons::button(ui, true, icon, 30.0)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        actions.push(Action::Execute(Command::ToggleReaderMode));
                    }
                }
                let star = places_ui::star_button(ui, can_bookmark, bookmarked);
                self.star_button = Some(star.rect);
                if star.clicked() {
                    actions.push(Action::Execute(Command::Bookmark));
                }
                self.url_bar(ui, actions);
            });
        });
        ui.add_space(3.0);
    }

    fn url_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let id = url_bar_id();
        let edit = egui::TextEdit::singleline(&mut self.url_text)
            .id(id)
            .hint_text("Введите поисковый запрос или адрес")
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(8.0, 6.0));
        let response = ui.add(edit);
        self.url_bar_rect = Some(response.rect);

        if self.focus_url_bar {
            self.focus_url_bar = false;
            response.request_focus();
            select_all(ui.ctx(), id, &self.url_text);
        }
        if response.changed() {
            self.editing = true;
            self.suggestion_selected = None;
            let query = self.url_text.trim().to_owned();
            if query.is_empty() {
                self.suggestions.clear();
            } else {
                actions.push(Action::Suggest(query));
            }
        }

        let show = response.has_focus() || response.lost_focus();
        let current = self.suggestions_query == self.url_text.trim();
        if self.editing && show && current {
            let ctx = ui.ctx().clone();
            let anchor = response.rect;
            if let Some(i) = places_ui::suggestions_popup(
                &ctx,
                anchor,
                &self.suggestions,
                &mut self.suggestion_selected,
            ) {
                self.choose_suggestion(i, actions);
                return;
            }
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                if let Some(i) = self
                    .suggestion_selected
                    .filter(|&i| i < self.suggestions.len())
                {
                    self.choose_suggestion(i, actions);
                    return;
                }
                let input = self.url_text.trim().to_owned();
                if !input.is_empty() {
                    actions.push(Action::Navigate(input));
                    actions.push(Action::FocusPage);
                }
                self.editing = false;
                self.suggestions.clear();
            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.editing = false;
                self.suggestions.clear();
            }
        }
    }
}

impl BrowserUi {
    fn choose_suggestion(&mut self, index: usize, actions: &mut Vec<Action>) {
        let url = self.suggestions[index].url.clone();
        actions.push(Action::NavigateSuggestion {
            query: self.suggestions_query.clone(),
            url,
        });
        actions.push(Action::FocusPage);
        self.editing = false;
        self.suggestions.clear();
        self.suggestion_selected = None;
    }
}

fn tab_button(
    ui: &mut egui::Ui,
    tab: &Tab,
    icon: Option<egui::TextureId>,
    selected: bool,
    width: f32,
    actions: &mut Vec<Action>,
) -> (Rect, bool) {
    let title = tab_title(tab);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());

    let response = ui.interact(
        rect,
        egui::Id::new(("klin-tab", tab.id)),
        Sense::click_and_drag(),
    );
    let visuals = ui.visuals().clone();
    let fill = if selected {
        visuals.widgets.active.weak_bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);

    let close_rect = Rect::from_center_size(
        rect.right_center() - Vec2::new(15.0, 0.0),
        Vec2::splat(20.0),
    );
    let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
    if close.hovered() {
        ui.painter().rect_filled(
            close_rect,
            CornerRadius::same(4),
            visuals.widgets.hovered.bg_fill,
        );
    }
    icons::paint(
        ui.painter(),
        close_rect.shrink(6.0),
        Icon::Close,
        visuals.text_color(),
    );

    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 10.0, rect.center().y - 8.0),
        Vec2::splat(16.0),
    );
    let loading = tab.load_status != LoadStatus::Complete;
    if loading {
        let time = ui.input(|i| i.time);
        icons::spinner(ui.painter(), icon_rect, visuals.selection.bg_fill, time);
    } else if let Some(texture) = icon {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture, icon_rect, uv, Color32::WHITE);
    } else {
        icons::paint(
            ui.painter(),
            icon_rect.shrink(1.0),
            Icon::Globe,
            visuals.weak_text_color(),
        );
    }

    let text_left = icon_rect.right() + 7.0;
    let text_width = (close_rect.left() - 4.0 - text_left).max(0.0);
    let label = title.clone();
    let color = if selected {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    };
    let mut job =
        egui::text::LayoutJob::simple_singleline(label, egui::FontId::proportional(13.0), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_width);
    let galley = ui.painter().layout_job(job);
    let pos = egui::pos2(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(rect).galley(pos, galley, color);

    if close.clicked() || response.middle_clicked() {
        actions.push(Action::CloseTab(tab.id));
    } else if !selected
        && !close.contains_pointer()
        && response.contains_pointer()
        && ui.input(|i| i.pointer.primary_pressed())
    {
        actions.push(Action::SelectTab(tab.id));
    }
    let dragged = response.dragged();
    if dragged {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else {
        response.on_hover_text(title);
    }
    (rect, dragged)
}

fn tab_title(tab: &Tab) -> String {
    let title = tab.title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    match tab.url.as_ref().map(display_url) {
        Some(url) if !url.is_empty() => url,
        _ => "Новая вкладка".to_owned(),
    }
}

fn status_panel(ui: &egui::Ui, page: Rect, text: &str) {
    let font = egui::FontId::proportional(12.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let size = galley.size() + Vec2::new(12.0, 6.0);
    let rect = Rect::from_min_size(page.left_bottom() - Vec2::new(0.0, size.y), size);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(2), ui.visuals().panel_fill);
    ui.painter().galley(
        rect.min + Vec2::new(6.0, 3.0),
        galley,
        ui.visuals().text_color(),
    );
}

fn crash_page(ui: &mut egui::Ui, reason: &str, actions: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);
        ui.heading("Ой. Вкладка упала.");
        ui.add_space(8.0);
        ui.label("Это могло произойти из-за ошибки в движке. Отчёт сохранён локально.");
        ui.add_space(4.0);
        ui.label(RichText::new(reason).small().weak());
        ui.add_space(16.0);
        if ui.button("Перезапустить эту вкладку").clicked() {
            actions.push(Action::Execute(Command::Reload));
        }
    });
}

fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        );
        state.cursor.set_char_range(Some(range));
        state.store(ctx, id);
    }
}
