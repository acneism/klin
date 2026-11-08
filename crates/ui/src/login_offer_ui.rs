use egui::{Key, Rect, RichText};
use klin_shared::TabId;
use klin_shared::logins::LoginInput;

use crate::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginOffer {
    pub tab: TabId,
    pub login: LoginInput,
    pub update: Option<String>,
}

pub(crate) struct OfferState {
    pub offer: LoginOffer,
    reveal: bool,
}

impl OfferState {
    pub fn new(offer: LoginOffer) -> Self {
        Self {
            offer,
            reveal: false,
        }
    }
}

pub(crate) fn panel(
    ctx: &egui::Context,
    anchor: Rect,
    state: &mut OfferState,
    actions: &mut Vec<Action>,
) -> bool {
    let width = 340.0;
    let pos = egui::pos2(anchor.left().max(4.0), anchor.bottom() + 4.0);
    let mut open = true;
    let site = crate::logins_ui::site_name(&state.offer.login.origin);
    let update = state.offer.update.is_some();
    egui::Area::new(egui::Id::new("klin-login-offer"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    let title = if update {
                        format!("Обновить логин для {site}?")
                    } else {
                        format!("Сохранить логин для {site}?")
                    };
                    ui.label(RichText::new(title).strong());
                    ui.add_space(8.0);
                    ui.label("Имя пользователя");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.offer.login.username)
                            .desired_width(width),
                    );
                    ui.label("Пароль");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.offer.login.password)
                            .password(!state.reveal)
                            .desired_width(width),
                    );
                    ui.checkbox(&mut state.reveal, "Показать пароль");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let (yes, no) = if update {
                            ("Обновить", "Не обновлять")
                        } else {
                            ("Сохранить", "Не сохранять")
                        };
                        let can_save = !state.offer.login.password.is_empty();
                        if ui.add_enabled(can_save, egui::Button::new(yes)).clicked() {
                            actions.push(Action::SaveLogin(state.offer.clone()));
                            open = false;
                        }
                        if ui.button(no).clicked() {
                            open = false;
                        }
                    });
                });
        });
    open && !ctx.input(|i| i.key_pressed(Key::Escape))
}
