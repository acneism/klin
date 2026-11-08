use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, ColorImage, RichText, Sense, TextureHandle, TextureId, TextureOptions, Vec2};
use klin_shared::Image;
use klin_shared::addons::AddonInfo;

use crate::Action;

pub const ADDONS_URL: &str = "about:addons";

#[derive(Default)]
pub(crate) struct AddonsState {
    pub addons: Vec<AddonInfo>,
    path: String,
    confirm_remove: Option<String>,
    icons: HashMap<String, (Arc<Image>, TextureHandle)>,
}

impl AddonsState {
    fn icon(&mut self, ctx: &egui::Context, uuid: &str, image: &Arc<Image>) -> Option<TextureId> {
        if let Some((cached, handle)) = self.icons.get(uuid)
            && Arc::ptr_eq(cached, image)
        {
            return Some(handle.id());
        }
        let size = [image.width as usize, image.height as usize];
        if size[0] == 0 || size[1] == 0 || image.rgba.len() != size[0] * size[1] * 4 {
            return None;
        }
        let handle = ctx.load_texture(
            format!("addon-{uuid}"),
            ColorImage::from_rgba_unmultiplied(size, &image.rgba),
            TextureOptions::LINEAR,
        );
        let id = handle.id();
        self.icons.insert(uuid.to_owned(), (image.clone(), handle));
        Some(id)
    }
}

pub(crate) fn toolbar_buttons(
    ui: &mut egui::Ui,
    state: &mut AddonsState,
    actions: &mut Vec<Action>,
) {
    let addons: Vec<AddonInfo> = state
        .addons
        .iter()
        .filter(|a| a.enabled && a.action.is_some())
        .cloned()
        .collect();
    for addon in addons.iter().rev() {
        let Some(action) = &addon.action else {
            continue;
        };
        let sense = if action.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), sense);
        if response.hovered() && action.enabled {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let inner = rect.shrink(7.0);
        let texture = action
            .icon
            .as_ref()
            .and_then(|icon| state.icon(ui.ctx(), &addon.uuid, icon));
        let tint = if action.enabled {
            Color32::WHITE
        } else {
            Color32::from_white_alpha(90)
        };
        match texture {
            Some(id) => {
                ui.painter().image(
                    id,
                    inner,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }
            None => {
                let letter: String = addon.name.chars().take(1).collect();
                ui.painter()
                    .rect_filled(inner, 3.0, Color32::from_rgb(0x59, 0x2a, 0xcb));
                ui.painter().text(
                    inner.center(),
                    egui::Align2::CENTER_CENTER,
                    letter.to_uppercase(),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
        }
        if !action.badge_text.is_empty() {
            let [r, g, b, a] = action.badge_color;
            let galley = ui.painter().layout_no_wrap(
                action.badge_text.clone(),
                egui::FontId::proportional(9.0),
                Color32::WHITE,
            );
            let width = galley.size().x + 4.0;
            let badge = egui::Rect::from_min_size(
                egui::pos2(rect.right() - width - 1.0, rect.bottom() - 12.0),
                Vec2::new(width, 11.0),
            );
            ui.painter()
                .rect_filled(badge, 3.0, Color32::from_rgba_unmultiplied(r, g, b, a));
            ui.painter().galley(
                egui::pos2(badge.left() + 2.0, badge.top()),
                galley,
                Color32::WHITE,
            );
        }
        if response.on_hover_text(&action.title).clicked() {
            actions.push(Action::AddonClicked(addon.uuid.clone()));
        }
    }
}

pub(crate) fn page(ui: &mut egui::Ui, state: &mut AddonsState, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.set_max_width(720.0);
                    ui.heading("Расширения");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Установить дополнение из файла…").clicked()
                        {
                            actions.push(Action::PickAddonFile);
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut state.path)
                                .hint_text("Путь к файлу .xpi или к папке с manifest.json")
                                .desired_width(480.0),
                        );
                        let path = state.path.trim().to_owned();
                        if ui
                            .add_enabled(!path.is_empty(), egui::Button::new("Установить"))
                            .clicked()
                        {
                            actions.push(Action::InstallAddon(path));
                            state.path.clear();
                        }
                    });
                    ui.add_space(12.0);
                    if state.addons.is_empty() {
                        ui.label(RichText::new("Расширения ещё не установлены.").weak());
                    }
                    let addons = state.addons.clone();
                    for addon in addons {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(700.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&addon.name).strong());
                                ui.label(RichText::new(&addon.version).weak());
                                if !addon.enabled {
                                    ui.label(RichText::new("(отключено)").weak());
                                }
                            });
                            if !addon.description.is_empty() {
                                ui.label(&addon.description);
                            }
                            for warning in &addon.warnings {
                                ui.label(RichText::new(warning).color(ui.visuals().warn_fg_color));
                            }
                            ui.horizontal(|ui| {
                                let mut enabled = addon.enabled;
                                if ui.checkbox(&mut enabled, "Включено").changed() {
                                    actions.push(Action::SetAddonEnabled {
                                        id: addon.id.clone(),
                                        enabled,
                                    });
                                }
                                if addon.enabled
                                    && addon.has_options
                                    && ui.button("Настройки").clicked()
                                {
                                    actions.push(Action::AddonOptions(addon.uuid.clone()));
                                }
                                if state.confirm_remove.as_deref() == Some(addon.id.as_str()) {
                                    ui.label(format!("Удалить «{}»?", addon.name));
                                    if ui.button("Удалить").clicked() {
                                        actions.push(Action::UninstallAddon(addon.id.clone()));
                                        state.confirm_remove = None;
                                    }
                                    if ui.button("Отмена").clicked() {
                                        state.confirm_remove = None;
                                    }
                                } else if ui.button("Удалить").clicked() {
                                    state.confirm_remove = Some(addon.id.clone());
                                }
                            });
                            ui.label(RichText::new(&addon.id).small().weak());
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        });
}
