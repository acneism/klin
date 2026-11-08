use egui::{Key, Rect, RichText};
use klin_core::browser::Tab;
use klin_shared::tracking::BlockCategory;

use crate::Action;
use crate::icons::{self, Icon};

pub(crate) fn shield_button(ui: &mut egui::Ui, tab: Option<&Tab>) -> Option<egui::Response> {
    let tab = tab?;
    let web = tab
        .url
        .as_ref()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https"));
    if !web {
        return None;
    }
    let blocked = !tab.blocked.is_empty();
    let icon = if blocked {
        Icon::ShieldActive
    } else {
        Icon::Shield
    };
    let hint = if blocked {
        "Klin блокирует части этой страницы, которые могут отслеживать вас"
    } else {
        "Трекеры на этом сайте не обнаружены"
    };
    Some(icons::button(ui, true, icon, 30.0).on_hover_text(hint))
}

pub(crate) fn panel(
    ctx: &egui::Context,
    anchor: Rect,
    tab: &Tab,
    actions: &mut Vec<Action>,
) -> bool {
    let width = 340.0;
    let pos = egui::pos2(anchor.left().max(4.0), anchor.bottom() + 4.0);
    let mut open = true;
    let host = tab
        .url
        .as_ref()
        .and_then(|u| u.host_str())
        .unwrap_or_default()
        .to_owned();
    let area = egui::Area::new(egui::Id::new("klin-protections-panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(RichText::new(format!("Защита на {host}")).strong());
                    ui.add_space(4.0);
                    ui.label("Улучшенная защита от отслеживания включена");
                    ui.separator();
                    if tab.blocked.is_empty() {
                        ui.label(RichText::new("Трекеры на этом сайте не обнаружены").weak());
                    } else {
                        ui.label(RichText::new("Заблокировано").strong());
                        let mut categories: Vec<BlockCategory> =
                            tab.blocked.iter().map(|(c, _)| *c).collect();
                        categories.sort();
                        categories.dedup();
                        for category in categories {
                            let hosts: Vec<&str> = tab
                                .blocked
                                .iter()
                                .filter(|(c, _)| *c == category)
                                .map(|(_, h)| h.as_str())
                                .collect();
                            egui::CollapsingHeader::new(format!(
                                "{} — {}",
                                category.label(),
                                hosts.len()
                            ))
                            .id_salt(category)
                            .show(ui, |ui| {
                                for host in hosts {
                                    ui.label(RichText::new(host).small());
                                }
                            });
                        }
                    }
                    ui.separator();
                    if ui.button("Параметры защиты").clicked() {
                        actions.push(Action::OpenInternal("about:preferences"));
                        open = false;
                    }
                });
        });
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .press_origin()
                .is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))
    });
    open && !pressed_outside && !ctx.input(|i| i.key_pressed(Key::Escape))
}
