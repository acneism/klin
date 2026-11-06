//! Интерфейс Klin на egui: вкладки, панель навигации, адресная строка, строка состояния.
//!
//! Крейт рисует состояние `klin_core::Browser` и возвращает действия пользователя
//! ([`Action`]). Движок он не вызывает: действия выполняет `app` через команды ядра.
//!
//! Строки интерфейса пока записаны здесь; в 0.2 их заменят строки Firefox в формате Fluent.

mod favicons;
mod icons;
mod requests;
mod url_display;

use egui::{Color32, CornerRadius, Key, Rect, RichText, Sense, Vec2};
use icons::Icon;
use klin_core::browser::Tab;
use klin_core::{Browser, Command};
use klin_shared::{LoadStatus, TabId, WindowId};

pub use url_display::display_url;

/// Что пользователь сделал в интерфейсе.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Execute(Command),
    SelectTab(TabId),
    CloseTab(TabId),
    /// Ввод адресной строки (Enter).
    Navigate(String),
    /// Клик по странице: фокус ввода переходит к ней.
    FocusPage,
    /// Ответ на запрос страницы (диалог, меню, список).
    Answer {
        tab: TabId,
        id: klin_shared::RequestId,
        reply: klin_core::Reply,
    },
}

/// Результат кадра интерфейса.
pub struct Frame {
    pub actions: Vec<Action>,
    /// Область страницы в точках egui.
    pub page_rect: Rect,
    /// Фокус клавиатуры у интерфейса (адресная строка, диалог), а не у страницы.
    pub ui_has_focus: bool,
    /// Страница не получает ввод: открыт диалог, меню или список.
    pub page_blocked: bool,
}

/// Состояние интерфейса одного окна.
#[derive(Default)]
pub struct BrowserUi {
    url_text: String,
    /// Пользователь редактирует адрес: не перезаписывать его адресом страницы.
    editing: bool,
    focus_url_bar: bool,
    shown_tab: Option<TabId>,
    requests: requests::RequestUi,
    favicons: favicons::Favicons,
}

fn url_bar_id() -> egui::Id {
    egui::Id::new("klin-url-bar")
}
const TAB_MIN_WIDTH: f32 = 76.0; // как min-width вкладки в Firefox
const TAB_MAX_WIDTH: f32 = 225.0; // как max-width вкладки в Firefox

impl BrowserUi {
    pub fn new() -> Self {
        Self::default()
    }

    /// Поставить фокус в адресную строку и выделить адрес (Ctrl+L, Alt+D, F6, новая вкладка).
    pub fn focus_url_bar(&mut self) {
        self.focus_url_bar = true;
    }

    /// Нарисовать окно внутри корневого `Ui` (из `Context::run_ui`).
    /// `paint_page` рисует страницу в отведённой ей области.
    pub fn show(
        &mut self,
        root: &mut egui::Ui,
        browser: &Browser,
        window: WindowId,
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
            self.nav_bar(ui, selected, &mut actions);
        });

        let mut page_rect = Rect::NOTHING;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                page_rect = ui.max_rect();
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

        let page_blocked = match selected {
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

    /// Адресная строка показывает адрес выбранной вкладки, пока пользователь его не правит.
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
            // Индикатор загрузки анимирован; 20 кадров в секунду достаточно.
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
                        for (index, &tab_id) in win.tabs.iter().enumerate() {
                            let Some(tab) = browser.tab(tab_id) else {
                                continue;
                            };
                            let icon = self.favicons.texture(ui.ctx(), tab);
                            tab_button(ui, tab, icon, index == win.selected, width, actions);
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

    fn nav_bar(&mut self, ui: &mut egui::Ui, tab: Option<&Tab>, actions: &mut Vec<Action>) {
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
            self.url_bar(ui, actions);
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

        if self.focus_url_bar {
            self.focus_url_bar = false;
            response.request_focus();
            select_all(ui.ctx(), id, &self.url_text);
        }
        if response.changed() {
            self.editing = true;
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                let input = self.url_text.trim().to_owned();
                if !input.is_empty() {
                    actions.push(Action::Navigate(input));
                    actions.push(Action::FocusPage);
                }
                self.editing = false;
            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                // Esc в адресной строке возвращает адрес страницы, как в Firefox.
                self.editing = false;
            }
        }
    }
}

fn tab_button(
    ui: &mut egui::Ui,
    tab: &Tab,
    icon: Option<egui::TextureId>,
    selected: bool,
    width: f32,
    actions: &mut Vec<Action>,
) {
    let title = tab_title(tab);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
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

    // Значок: при загрузке — индикатор, без значка — значок по умолчанию, как в Firefox.
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

    // Заголовок рисуем сами: виджет-надпись занял бы место в ряду ещё раз.
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
        // Средняя кнопка мыши закрывает вкладку, как в Firefox.
        actions.push(Action::CloseTab(tab.id));
    } else if response.clicked() {
        actions.push(Action::SelectTab(tab.id));
    }
    response.on_hover_text(title);
}

/// Заголовок вкладки: заголовок страницы, иначе адрес, иначе «Новая вкладка».
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

/// Адрес ссылки под курсором в левом нижнем углу страницы.
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

/// Страница сбоя вкладки — по образцу about:tabcrashed в Firefox.
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
