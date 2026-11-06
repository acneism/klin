//! Запросы страницы: диалоги alert/confirm/prompt, вход на сайт, разрешения,
//! контекстное меню и выпадающие списки.
//!
//! Как в Firefox, диалоги модальны для вкладки: страница затемнена и не получает ввод,
//! но остальная часть окна работает. Запрос разрешения не модален (панель под адресной строкой).

use std::collections::HashMap;

use egui::{Align2, Color32, Key, Rect, RichText, Vec2, pos2, vec2};
use klin_core::browser::Tab;
use klin_core::{MenuAction, MenuItem, PageRequest, PermissionFeature, Reply, RequestKind};
use klin_shared::RequestId;

use crate::Action;

#[derive(Default)]
pub(crate) struct RequestUi {
    text: HashMap<RequestId, String>,
    password: HashMap<RequestId, String>,
    selection: HashMap<RequestId, Vec<usize>>,
}

impl RequestUi {
    /// Нарисовать первый запрос вкладки. Возвращает `true`, если страница заблокирована.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        tab: &Tab,
        page: Rect,
        actions: &mut Vec<Action>,
    ) -> bool {
        let live: Vec<RequestId> = tab.requests.iter().map(|r| r.id).collect();
        self.text.retain(|id, _| live.contains(id));
        self.password.retain(|id, _| live.contains(id));
        self.selection.retain(|id, _| live.contains(id));

        let Some(request) = tab.requests.first() else {
            return false;
        };
        let host = tab
            .url
            .as_ref()
            .and_then(|u| u.host_str())
            .unwrap_or_default()
            .to_owned();
        let mut reply = None;
        let blocking = match &request.kind {
            RequestKind::Permission { feature } => {
                reply = permission_panel(ctx, page, &host, *feature);
                false
            }
            RequestKind::ContextMenu { x, y, items, .. } => {
                let pos = page.min + vec2(*x as f32, *y as f32) / ctx.pixels_per_point();
                reply = context_menu(ctx, pos, items);
                true
            }
            RequestKind::Select {
                rect,
                options,
                multiple,
                selected,
            } => {
                let ppp = ctx.pixels_per_point();
                let anchor = Rect::from_min_size(
                    page.min + vec2(rect[0] as f32, rect[1] as f32) / ppp,
                    vec2(rect[2] as f32, rect[3] as f32) / ppp,
                );
                let chosen = self
                    .selection
                    .entry(request.id)
                    .or_insert_with(|| selected.clone());
                reply = select_popup(ctx, anchor, options, *multiple, chosen);
                true
            }
            RequestKind::FilePicker { .. } => {
                // Системный диалог открыт приложением; страница ждёт.
                dim(ctx, page);
                true
            }
            _ => {
                dim(ctx, page);
                reply = self.dialog(ctx, page, &host, request);
                true
            }
        };
        if let Some(reply) = reply {
            actions.push(Action::Answer {
                tab: tab.id,
                id: request.id,
                reply,
            });
        }
        blocking
    }

    fn dialog(
        &mut self,
        ctx: &egui::Context,
        page: Rect,
        host: &str,
        request: &PageRequest,
    ) -> Option<Reply> {
        let width = 420.0_f32.min(page.width() - 32.0).max(200.0);
        let pos = pos2(page.center().x - width / 2.0, page.top() + 48.0);
        let mut reply = None;
        let (enter, escape) =
            ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        egui::Area::new(egui::Id::new(("klin-dialog", request.id.get())))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let title = if host.is_empty() {
                            "Эта страница сообщает".to_owned()
                        } else {
                            host.to_owned()
                        };
                        ui.label(RichText::new(title).strong());
                        ui.add_space(8.0);
                        match &request.kind {
                            RequestKind::Alert { message } => {
                                message_text(ui, message);
                                ui.add_space(12.0);
                                if buttons(ui, None) == Some(true) || enter || escape {
                                    reply = Some(Reply::Ok);
                                }
                            }
                            RequestKind::Confirm { message } => {
                                message_text(ui, message);
                                ui.add_space(12.0);
                                match buttons(ui, Some("Отмена")) {
                                    Some(true) => reply = Some(Reply::Ok),
                                    Some(false) => reply = Some(Reply::Dismiss),
                                    None if enter => reply = Some(Reply::Ok),
                                    None if escape => reply = Some(Reply::Dismiss),
                                    None => {}
                                }
                            }
                            RequestKind::Prompt { message, default } => {
                                message_text(ui, message);
                                ui.add_space(8.0);
                                let text = self
                                    .text
                                    .entry(request.id)
                                    .or_insert_with(|| default.clone());
                                let field = ui.add(
                                    egui::TextEdit::singleline(text).desired_width(f32::INFINITY),
                                );
                                if !field.has_focus() && !field.lost_focus() {
                                    field.request_focus();
                                }
                                ui.add_space(12.0);
                                let text = text.clone();
                                match buttons(ui, Some("Отмена")) {
                                    Some(true) => reply = Some(Reply::Text(text)),
                                    Some(false) => reply = Some(Reply::Dismiss),
                                    None if enter => reply = Some(Reply::Text(text)),
                                    None if escape => reply = Some(Reply::Dismiss),
                                    None => {}
                                }
                            }
                            RequestKind::Authentication { url, for_proxy } => {
                                let what = if *for_proxy {
                                    "Прокси-сервер"
                                } else {
                                    "Этот сайт"
                                };
                                let origin = url.host_str().unwrap_or(url.as_str());
                                ui.label(format!(
                                    "{what} {origin} запрашивает имя пользователя и пароль."
                                ));
                                ui.add_space(8.0);
                                let user = self.text.entry(request.id).or_default();
                                ui.label("Имя пользователя");
                                let user_field = ui.add(
                                    egui::TextEdit::singleline(user).desired_width(f32::INFINITY),
                                );
                                let user = user.clone();
                                let pass = self.password.entry(request.id).or_default();
                                ui.label("Пароль");
                                ui.add(
                                    egui::TextEdit::singleline(pass)
                                        .password(true)
                                        .desired_width(f32::INFINITY),
                                );
                                if user.is_empty() && pass.is_empty() && !user_field.has_focus() {
                                    user_field.request_focus();
                                }
                                let creds = Reply::Credentials {
                                    username: user,
                                    password: pass.clone(),
                                };
                                ui.add_space(12.0);
                                match buttons(ui, Some("Отмена")) {
                                    Some(true) => reply = Some(creds),
                                    Some(false) => reply = Some(Reply::Dismiss),
                                    None if enter => reply = Some(creds),
                                    None if escape => reply = Some(Reply::Dismiss),
                                    None => {}
                                }
                            }
                            _ => {}
                        }
                    });
            });
        reply
    }
}

fn message_text(ui: &mut egui::Ui, message: &str) {
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            ui.add(egui::Label::new(message).wrap());
        });
}

/// «ОК» и, если задано, кнопка отмены. `Some(true)` — ОК, `Some(false)` — отмена.
fn buttons(ui: &mut egui::Ui, cancel: Option<&str>) -> Option<bool> {
    let mut result = None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add(egui::Button::new("ОК").min_size(vec2(80.0, 26.0)))
            .clicked()
        {
            result = Some(true);
        }
        if let Some(cancel) = cancel
            && ui
                .add(egui::Button::new(cancel).min_size(vec2(80.0, 26.0)))
                .clicked()
        {
            result = Some(false);
        }
    });
    result
}

/// Затемнить страницу под модальным диалогом.
fn dim(ctx: &egui::Context, page: Rect) {
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("klin-dim"));
    ctx.layer_painter(layer)
        .rect_filled(page, 0.0, Color32::from_black_alpha(110));
}

fn permission_panel(
    ctx: &egui::Context,
    page: Rect,
    host: &str,
    feature: PermissionFeature,
) -> Option<Reply> {
    let text = match feature {
        PermissionFeature::Geolocation => {
            format!("Разрешить {host} доступ к вашему местоположению?")
        }
        PermissionFeature::Notifications => format!("Разрешить {host} отправлять уведомления?"),
        PermissionFeature::Camera => format!("Разрешить {host} использовать вашу камеру?"),
        PermissionFeature::Microphone => format!("Разрешить {host} использовать ваш микрофон?"),
        PermissionFeature::PersistentStorage => {
            format!("Разрешить {host} хранить данные в постоянном хранилище?")
        }
        PermissionFeature::Other => format!("Разрешить {host} доступ к этой возможности?"),
    };
    let mut reply = None;
    egui::Area::new(egui::Id::new("klin-permission"))
        .order(egui::Order::Foreground)
        .fixed_pos(page.min + vec2(8.0, 4.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.add(egui::Label::new(text).wrap());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Блокировать").clicked() {
                            reply = Some(Reply::Deny);
                        }
                        if ui.button("Разрешить").clicked() {
                            reply = Some(Reply::Allow);
                        }
                    });
                });
        });
    if reply.is_none() && ctx.input(|i| i.key_pressed(Key::Escape)) {
        // Esc закрывает панель без решения — для страницы это отказ, как «Не сейчас».
        reply = Some(Reply::Dismiss);
    }
    reply
}

/// Новое нажатие кнопки мыши вне всплывающего окна. Отпускание кнопки не считается:
/// меню и списки открываются по нажатию, и его отпускание пришло бы «мимо».
fn pressed_outside(ctx: &egui::Context, popup: Rect) -> bool {
    ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|p| !popup.contains(p))
    })
}

/// Подписи пунктов — как в контекстном меню Firefox.
pub(crate) fn menu_label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::Back => "Назад",
        MenuAction::Forward => "Вперёд",
        MenuAction::Reload => "Обновить",
        MenuAction::CopyLink => "Копировать ссылку",
        MenuAction::OpenLinkInNewTab => "Открыть ссылку в новой вкладке",
        MenuAction::CopyImageLink => "Копировать ссылку на изображение",
        MenuAction::OpenImageInNewTab => "Открыть изображение в новой вкладке",
        MenuAction::Cut => "Вырезать",
        MenuAction::Copy => "Копировать",
        MenuAction::Paste => "Вставить",
        MenuAction::SelectAll => "Выделить всё",
    }
}

fn context_menu(ctx: &egui::Context, pos: egui::Pos2, items: &[MenuItem]) -> Option<Reply> {
    let mut reply = None;
    let area = egui::Area::new(egui::Id::new("klin-context-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(220.0);
                for item in items {
                    match item {
                        MenuItem::Separator => {
                            ui.separator();
                        }
                        MenuItem::Action { action, enabled } => {
                            let button = egui::Button::new(menu_label(*action)).frame(false);
                            let response = ui.add_enabled(*enabled, button);
                            if response.clicked() {
                                reply = Some(Reply::Menu(*action));
                            }
                        }
                    }
                }
            });
        });
    if reply.is_none()
        && (pressed_outside(ctx, area.response.rect) || ctx.input(|i| i.key_pressed(Key::Escape)))
    {
        reply = Some(Reply::Dismiss);
    }
    reply
}

fn select_popup(
    ctx: &egui::Context,
    anchor: Rect,
    options: &[klin_core::SelectOption],
    multiple: bool,
    chosen: &mut Vec<usize>,
) -> Option<Reply> {
    let mut reply = None;
    let area = egui::Area::new(egui::Id::new("klin-select"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom())
        .pivot(Align2::LEFT_TOP)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                let width = anchor.width().max(120.0);
                ui.set_min_width(width);
                ui.set_max_width(width.max(320.0));
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        let mut group: Option<&String> = None;
                        for option in options {
                            if option.group.as_ref() != group {
                                group = option.group.as_ref();
                                if let Some(label) = group {
                                    ui.label(RichText::new(label).strong());
                                }
                            }
                            let indent = if option.group.is_some() { "    " } else { "" };
                            let is_chosen = chosen.contains(&option.id);
                            let label = egui::Button::selectable(
                                is_chosen,
                                format!("{indent}{}", option.label),
                            )
                            .min_size(Vec2::new(ui.available_width(), 0.0));
                            if ui.add_enabled(!option.disabled, label).clicked() {
                                if multiple {
                                    if is_chosen {
                                        chosen.retain(|&id| id != option.id);
                                    } else {
                                        chosen.push(option.id);
                                    }
                                } else {
                                    reply = Some(Reply::Select(vec![option.id]));
                                }
                            }
                        }
                    });
            });
        });
    if reply.is_none()
        && (pressed_outside(ctx, area.response.rect) || ctx.input(|i| i.key_pressed(Key::Escape)))
    {
        reply = Some(if multiple {
            Reply::Select(chosen.clone())
        } else {
            Reply::Dismiss
        });
    }
    reply
}
