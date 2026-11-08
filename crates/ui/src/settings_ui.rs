use std::collections::HashMap;

use egui::{RichText, Vec2};
use klin_core::Command;
use klin_core::prefs::{self, PrefValue, UserPrefs};

use crate::Action;
use crate::icons::{self, Icon};

pub const PREFERENCES_URL: &str = "about:preferences";
pub const CONFIG_URL: &str = "about:config";

#[derive(Default)]
pub(crate) struct SettingsState {
    homepage: Option<String>,
    download_dir: Option<String>,
    confirm_clear_history: bool,
    config_accepted: bool,
    config_filter: String,
    config_edits: HashMap<String, String>,
    menu_open: bool,
}

pub(crate) fn menu_button(
    ui: &mut egui::Ui,
    state: &mut SettingsState,
    zoom: Option<f32>,
    actions: &mut Vec<Action>,
) {
    let button = icons::button(ui, true, Icon::Menu, 30.0).on_hover_text("Открыть меню приложения");
    if button.clicked() {
        state.menu_open = !state.menu_open;
    }
    if !state.menu_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let width = 300.0;
    let pos = egui::pos2(
        (button.rect.right() - width).max(4.0),
        button.rect.bottom() + 4.0,
    );
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("klin-app-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(&ctx, |ui| {
            egui::Frame::menu(ui.style())
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Новая вкладка",
                        "Ctrl+T",
                        Action::Execute(Command::NewTab),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Новое окно",
                        "Ctrl+N",
                        Action::Execute(Command::NewWindow),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Новое приватное окно",
                        "Ctrl+Shift+P",
                        Action::Execute(Command::NewPrivateWindow),
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "История",
                        "Ctrl+H",
                        Action::ToggleHistorySidebar,
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Загрузки",
                        "Ctrl+J",
                        Action::Execute(Command::ShowDownloads),
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Сохранить страницу как…",
                        "Ctrl+S",
                        Action::Execute(Command::SavePage),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Найти на странице…",
                        "Ctrl+F",
                        Action::Execute(Command::Find),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Масштаб");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(" + ")
                                .on_hover_text("Увеличить (Ctrl++)")
                                .clicked()
                            {
                                actions.push(Action::Execute(Command::ZoomIn));
                            }
                            let percent =
                                zoom.map_or("—".to_owned(), |z| format!("{:.0}%", z * 100.0));
                            if ui
                                .button(percent)
                                .on_hover_text("Сбросить (Ctrl+0)")
                                .clicked()
                            {
                                actions.push(Action::Execute(Command::ZoomReset));
                            }
                            if ui
                                .button(" − ")
                                .on_hover_text("Уменьшить (Ctrl+-)")
                                .clicked()
                            {
                                actions.push(Action::Execute(Command::ZoomOut));
                            }
                        });
                    });
                    ui.separator();
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Дополнения и темы",
                        "Ctrl+Shift+A",
                        Action::OpenInternal(crate::addons_ui::ADDONS_URL),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Пароли",
                        "",
                        Action::OpenInternal(crate::logins_ui::LOGINS_URL),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Настройки",
                        "",
                        Action::OpenInternal(PREFERENCES_URL),
                    );
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Импорт данных из Firefox…",
                        "",
                        Action::ImportFromFirefox,
                    );
                    ui.separator();
                    menu_item(
                        ui,
                        actions,
                        &mut close,
                        "Выход",
                        "Ctrl+Shift+Q",
                        Action::Execute(Command::Quit),
                    );
                });
        });
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .press_origin()
                .is_some_and(|p| !area.response.rect.contains(p) && !button.rect.contains(p))
    });
    if close || pressed_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.menu_open = false;
    }
}

fn menu_item(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    close: &mut bool,
    label: &str,
    shortcut: &str,
    action: Action,
) {
    let response = ui.add(
        egui::Button::new(label)
            .shortcut_text(RichText::new(shortcut).weak())
            .frame(false)
            .min_size(Vec2::new(ui.available_width(), 24.0)),
    );
    if response.clicked() {
        actions.push(action);
        *close = true;
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(title).size(20.0).strong());
    ui.add_space(6.0);
}

fn group(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(2.0);
}

fn text_pref(
    ui: &mut egui::Ui,
    edit: &mut Option<String>,
    current: String,
    hint: &str,
    name: &'static str,
    actions: &mut Vec<Action>,
) {
    let text = edit.get_or_insert_with(|| current.clone());
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(420.0),
    );
    if response.lost_focus() {
        let value = text.trim().to_owned();
        if value != current {
            if value.is_empty() {
                actions.push(Action::ResetPref(name.into()));
            } else {
                actions.push(Action::SetPref(name.into(), PrefValue::String(value)));
            }
        }
        *edit = None;
    }
}

pub(crate) fn preferences_page(
    ui: &mut egui::Ui,
    prefs: &UserPrefs,
    state: &mut SettingsState,
    actions: &mut Vec<Action>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.set_max_width(640.0);
                    ui.label(RichText::new("Настройки").size(26.0));

                    section(ui, "Основные");
                    group(ui, "Запуск");
                    let mut restore = prefs.startup_page() == 3;
                    if ui
                        .checkbox(&mut restore, "Открывать предыдущие окна и вкладки")
                        .changed()
                    {
                        actions.push(Action::SetPref(
                            prefs::STARTUP_PAGE.into(),
                            PrefValue::Int(if restore { 3 } else { 1 }),
                        ));
                    }

                    group(ui, "Домашняя страница");
                    text_pref(
                        ui,
                        &mut state.homepage,
                        prefs.homepage(),
                        prefs::HOMEPAGE,
                        prefs::HOMEPAGE_PREF,
                        actions,
                    );

                    group(ui, "Загрузки");
                    ui.label("Путь для сохранения файлов");
                    text_pref(
                        ui,
                        &mut state.download_dir,
                        prefs.string(prefs::DOWNLOAD_DIR, ""),
                        "Папка «Загрузки»",
                        prefs::DOWNLOAD_DIR,
                        actions,
                    );

                    let mut open_pdf = !prefs.known_bool(prefs::PDFJS_DISABLED);
                    if ui
                        .checkbox(&mut open_pdf, "Открывать PDF-файлы в Klin")
                        .changed()
                    {
                        actions.push(Action::SetPref(
                            prefs::PDFJS_DISABLED.into(),
                            PrefValue::Bool(!open_pdf),
                        ));
                    }

                    group(ui, "Импорт данных браузера");
                    if ui.button("Импорт данных из Firefox…").clicked() {
                        actions.push(Action::ImportFromFirefox);
                    }

                    section(ui, "Приватность и защита");
                    group(ui, "Улучшенная защита от отслеживания");
                    ui.label(
                        RichText::new(
                            "Трекеры следят за вами в Интернете, собирая информацию о ваших \
                             привычках и интересах. Klin блокирует многие из них.",
                        )
                        .weak(),
                    );
                    let category = prefs.string(prefs::CONTENT_BLOCKING_CATEGORY, "standard");
                    let standard = ui
                        .radio(category == "standard", "Стандартная")
                        .on_hover_text(
                            "Баланс защиты и производительности: трекеры социальных сетей, \
                             криптомайнеры и сборщики отпечатков; отслеживающее содержимое \
                             — в приватных окнах",
                        );
                    if standard.clicked() && category != "standard" {
                        actions.push(Action::ResetPref(prefs::CONTENT_BLOCKING_CATEGORY.into()));
                        for name in prefs::STRICT_PREFS {
                            actions.push(Action::ResetPref((*name).into()));
                        }
                    }
                    let strict = ui.radio(category == "strict", "Строгая").on_hover_text(
                        "Сильнее защищает, но может нарушить работу некоторых сайтов: \
                         отслеживающее содержимое блокируется во всех окнах",
                    );
                    if strict.clicked() && category != "strict" {
                        actions.push(Action::SetPref(
                            prefs::CONTENT_BLOCKING_CATEGORY.into(),
                            PrefValue::String("strict".into()),
                        ));
                        for name in prefs::STRICT_PREFS {
                            actions.push(Action::SetPref((*name).into(), PrefValue::Bool(true)));
                        }
                    }
                    if category != "standard" && category != "strict" {
                        let _ = ui.radio(true, "Особая (изменена в about:config)");
                    }

                    group(ui, "Логины и пароли");
                    let mut remember = prefs.known_bool(prefs::REMEMBER_SIGNONS);
                    if ui
                        .checkbox(
                            &mut remember,
                            "Запрашивать сохранение логинов и паролей для сайтов",
                        )
                        .changed()
                    {
                        actions.push(Action::SetPref(
                            prefs::REMEMBER_SIGNONS.into(),
                            PrefValue::Bool(remember),
                        ));
                    }
                    let mut autofill = prefs.known_bool(prefs::AUTOFILL_FORMS);
                    if ui
                        .checkbox(&mut autofill, "Автоматически заполнять логины и пароли")
                        .changed()
                    {
                        actions.push(Action::SetPref(
                            prefs::AUTOFILL_FORMS.into(),
                            PrefValue::Bool(autofill),
                        ));
                    }
                    if ui.button("Сохранённые логины…").clicked() {
                        actions.push(Action::OpenInternal(crate::logins_ui::LOGINS_URL));
                    }

                    group(ui, "История");
                    let mut remember = prefs.history_enabled();
                    if ui
                        .checkbox(&mut remember, "Запоминать историю посещений")
                        .changed()
                    {
                        actions.push(Action::SetPref(
                            prefs::HISTORY_ENABLED.into(),
                            PrefValue::Bool(remember),
                        ));
                    }
                    if state.confirm_clear_history {
                        ui.horizontal(|ui| {
                            ui.label("Удалить всю историю посещений?");
                            if ui.button("Удалить").clicked() {
                                actions.push(Action::ClearHistory);
                                state.confirm_clear_history = false;
                            }
                            if ui.button("Отмена").clicked() {
                                state.confirm_clear_history = false;
                            }
                        });
                    } else if ui.button("Удалить историю…").clicked() {
                        state.confirm_clear_history = true;
                    }

                    section(ui, "Дополнительно");
                    if ui.link("Расширенные настройки (about:config)").clicked()
                    {
                        actions.push(Action::OpenInternal(CONFIG_URL));
                    }
                    ui.add_space(24.0);
                });
            });
        });
}

fn value_text(value: &PrefValue) -> String {
    match value {
        PrefValue::Bool(b) => b.to_string(),
        PrefValue::Int(i) => i.to_string(),
        PrefValue::String(s) => s.clone(),
    }
}

pub(crate) fn config_page(
    ui: &mut egui::Ui,
    prefs: &UserPrefs,
    state: &mut SettingsState,
    actions: &mut Vec<Action>,
) {
    if !state.config_accepted {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 4.0);
            ui.label(RichText::new("Будьте осторожны").size(24.0).strong());
            ui.add_space(10.0);
            ui.label("Изменение расширенных настроек может повлиять на производительность или безопасность браузера.");
            ui.add_space(16.0);
            if ui.button("Принять риск и продолжить").clicked() {
                state.config_accepted = true;
            }
        });
        return;
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(&mut state.config_filter)
                .hint_text("Поиск по имени настройки")
                .desired_width(420.0),
        );
    });
    ui.add_space(6.0);
    let filter = state.config_filter.to_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.indent("klin-config-indent", |ui| {
                config_grid(ui, prefs, state, &filter, actions)
            });
        });
}

fn config_grid(
    ui: &mut egui::Ui,
    prefs: &UserPrefs,
    state: &mut SettingsState,
    filter: &str,
    actions: &mut Vec<Action>,
) {
    egui::Grid::new("klin-config")
        .striped(true)
        .num_columns(3)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for name in prefs.names() {
                if !filter.is_empty() && !name.to_lowercase().contains(filter) {
                    continue;
                }
                let Some(value) = prefs.value_or_default(&name) else {
                    continue;
                };
                let modified = prefs.is_user_set(&name);
                let label = if modified {
                    RichText::new(&name).strong()
                } else {
                    RichText::new(&name)
                };
                ui.label(label);
                match &value {
                    PrefValue::Bool(b) => {
                        ui.label(b.to_string());
                    }
                    other => {
                        let text = state
                            .config_edits
                            .entry(name.clone())
                            .or_insert_with(|| value_text(other));
                        let response =
                            ui.add_sized([320.0, 20.0], egui::TextEdit::singleline(text));
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let parsed = match other {
                                PrefValue::Int(_) => text.trim().parse().ok().map(PrefValue::Int),
                                _ => Some(PrefValue::String(text.clone())),
                            };
                            if let Some(v) = parsed {
                                actions.push(Action::SetPref(name.clone(), v));
                            }
                            state.config_edits.remove(&name);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if let PrefValue::Bool(b) = value
                        && ui.button("Переключить").clicked()
                    {
                        actions.push(Action::SetPref(name.clone(), PrefValue::Bool(!b)));
                    }
                    if modified && ui.button("Сбросить").clicked() {
                        actions.push(Action::ResetPref(name.clone()));
                        state.config_edits.remove(&name);
                    }
                });
                ui.end_row();
            }
        });
}
