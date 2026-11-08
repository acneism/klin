use egui::{RichText, Vec2};
use klin_shared::logins::{LoginInput, SavedLogin};

use crate::Action;

pub const LOGINS_URL: &str = "about:logins";

#[derive(Default)]
enum Editing {
    #[default]
    None,
    Existing {
        username: String,
        password: String,
    },
    New {
        origin: String,
        username: String,
        password: String,
    },
}

#[derive(Default)]
pub(crate) struct LoginsState {
    pub requested: bool,
    pub logins: Vec<SavedLogin>,
    filter: String,
    selected: Option<String>,
    reveal: bool,
    editing: Editing,
    confirm_delete: bool,
    confirm_delete_all: bool,
}

impl LoginsState {
    pub fn set_logins(&mut self, logins: Vec<SavedLogin>) {
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !logins.iter().any(|l| &l.id == id))
        {
            self.selected = None;
        }
        self.logins = logins;
    }
}

pub(crate) fn site_name(origin: &str) -> String {
    let host = klin_shared::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| origin.to_owned());
    host.strip_prefix("www.").map(str::to_owned).unwrap_or(host)
}

fn date(ms: i64) -> String {
    if ms <= 0 {
        return "—".into();
    }
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}.{y}")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn copy_button(ui: &mut egui::Ui, text: &str, label: &str) {
    if ui.button(label).clicked() {
        ui.ctx().copy_text(text.to_owned());
    }
}

pub(crate) fn show(ui: &mut egui::Ui, state: &mut LoginsState, actions: &mut Vec<Action>) {
    if !state.requested {
        state.requested = true;
        actions.push(Action::LoadLogins);
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(280.0);
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut state.filter)
                    .hint_text("Поиск паролей")
                    .desired_width(270.0),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Создать пароль").clicked() {
                    state.selected = None;
                    state.reveal = false;
                    state.editing = Editing::New {
                        origin: String::new(),
                        username: String::new(),
                        password: String::new(),
                    };
                }
                if !state.logins.is_empty() {
                    if state.confirm_delete_all {
                        if ui.button("Удалить все").clicked() {
                            actions.push(Action::DeleteAllLogins);
                            state.confirm_delete_all = false;
                        }
                        if ui.button("Отмена").clicked() {
                            state.confirm_delete_all = false;
                        }
                    } else if ui.button("Удалить все пароли…").clicked() {
                        state.confirm_delete_all = true;
                    }
                }
            });
            ui.add_space(6.0);
            let filter = state.filter.to_lowercase();
            let visible: Vec<&SavedLogin> = state
                .logins
                .iter()
                .filter(|l| {
                    filter.is_empty()
                        || l.origin.to_lowercase().contains(&filter)
                        || l.username.to_lowercase().contains(&filter)
                })
                .collect();
            ui.label(
                RichText::new(format!("Пароли: {}", visible.len()))
                    .small()
                    .weak(),
            );
            egui::ScrollArea::vertical()
                .id_salt("logins-list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for login in visible {
                        let selected = state.selected.as_deref() == Some(login.id.as_str());
                        let user = if login.username.is_empty() {
                            "(нет имени пользователя)"
                        } else {
                            login.username.as_str()
                        };
                        let text = format!("{}\n{user}", site_name(&login.origin));
                        if ui
                            .add_sized(
                                Vec2::new(270.0, 36.0),
                                egui::Button::selectable(selected, text),
                            )
                            .clicked()
                        {
                            state.selected = Some(login.id.clone());
                            state.reveal = false;
                            state.editing = Editing::None;
                            state.confirm_delete = false;
                        }
                    }
                });
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.add_space(8.0);
            details(ui, state, actions);
        });
    });
}

fn details(ui: &mut egui::Ui, state: &mut LoginsState, actions: &mut Vec<Action>) {
    if let Editing::New {
        origin,
        username,
        password,
    } = &mut state.editing
    {
        ui.heading("Создать пароль");
        ui.add_space(8.0);
        ui.label("Адрес сайта");
        ui.add(egui::TextEdit::singleline(origin).hint_text("https://www.example.com"));
        ui.label("Имя пользователя");
        ui.text_edit_singleline(username);
        ui.label("Пароль");
        ui.add(egui::TextEdit::singleline(password).password(true));
        ui.add_space(8.0);
        let parsed = klin_shared::Url::parse(origin.trim())
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"));
        let mut done = false;
        ui.horizontal(|ui| {
            let can_save = parsed.is_some() && !password.is_empty();
            if ui
                .add_enabled(can_save, egui::Button::new("Сохранить"))
                .clicked()
                && let Some(url) = &parsed
            {
                let origin = url.origin().ascii_serialization();
                actions.push(Action::AddLogin(LoginInput {
                    form_action_origin: Some(origin.clone()),
                    origin,
                    username: username.clone(),
                    password: password.clone(),
                    ..Default::default()
                }));
                done = true;
            }
            if ui.button("Отмена").clicked() {
                done = true;
            }
        });
        if done {
            state.editing = Editing::None;
        }
        return;
    }

    let Some(login) = state
        .selected
        .as_ref()
        .and_then(|id| state.logins.iter().find(|l| &l.id == id))
        .cloned()
    else {
        if state.logins.is_empty() {
            ui.heading("Здесь будут ваши пароли");
            ui.label("Klin сохраняет пароли, которые вы вводите на сайтах, и подставляет их при следующем входе. Пароли можно перенести из Firefox через «Импорт данных из Firefox».");
        } else {
            ui.label(RichText::new("Выберите сохранённый пароль слева").weak());
        }
        return;
    };

    ui.heading(site_name(&login.origin));
    if ui.link(&login.origin).clicked()
        && let Ok(url) = klin_shared::Url::parse(&login.origin)
    {
        actions.push(Action::OpenBookmark { url, new_tab: true });
    }
    ui.add_space(10.0);

    let mut finished = false;
    if let Editing::Existing { username, password } = &mut state.editing {
        ui.label("Имя пользователя");
        ui.text_edit_singleline(username);
        ui.label("Пароль");
        ui.add(egui::TextEdit::singleline(password).password(!state.reveal));
        ui.checkbox(&mut state.reveal, "Показать пароль");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!password.is_empty(), egui::Button::new("Сохранить"))
                .clicked()
            {
                actions.push(Action::UpdateLogin {
                    id: login.id.clone(),
                    username: username.clone(),
                    password: password.clone(),
                });
                finished = true;
            }
            if ui.button("Отмена").clicked() {
                finished = true;
            }
        });
    } else {
        ui.label(RichText::new("Имя пользователя").small().weak());
        ui.horizontal(|ui| {
            ui.label(&login.username);
            copy_button(ui, &login.username, "Копировать");
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Пароль").small().weak());
        ui.horizontal(|ui| {
            let shown = if state.reveal {
                login.password.clone()
            } else {
                "•".repeat(login.password.chars().count().clamp(8, 16))
            };
            ui.label(RichText::new(shown).monospace());
            let toggle = if state.reveal {
                "Скрыть"
            } else {
                "Показать"
            };
            if ui.button(toggle).clicked() {
                state.reveal = !state.reveal;
            }
            copy_button(ui, &login.password, "Копировать");
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Изменить").clicked() {
                state.editing = Editing::Existing {
                    username: login.username.clone(),
                    password: login.password.clone(),
                };
            }
            if state.confirm_delete {
                ui.label("Удалить этот пароль?");
                if ui.button("Удалить").clicked() {
                    actions.push(Action::DeleteLogin(login.id.clone()));
                    state.confirm_delete = false;
                    state.selected = None;
                }
                if ui.button("Отмена").clicked() {
                    state.confirm_delete = false;
                }
            } else if ui.button("Удалить").clicked() {
                state.confirm_delete = true;
            }
        });
        ui.add_space(14.0);
        let meta = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.label(RichText::new(format!("{label}: {value}")).small().weak());
        };
        meta(ui, "Создан", date(login.time_created));
        meta(ui, "Изменён", date(login.time_password_changed));
        meta(ui, "Последнее использование", date(login.time_last_used));
    }
    if finished {
        state.editing = Editing::None;
        state.reveal = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates_and_site_names() {
        assert_eq!(date(0), "—");
        assert_eq!(date(1_700_000_000_000), "14.11.2023");
        assert_eq!(date(951_782_400_000), "29.02.2000");
        assert_eq!(site_name("https://www.example.com"), "example.com");
        assert_eq!(
            site_name("https://accounts.example.com:8443"),
            "accounts.example.com"
        );
    }
}
