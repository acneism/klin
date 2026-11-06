use egui::{Key, Rect, RichText, Vec2};
use klin_shared::Url;

use crate::Action;
use crate::icons::{self, Icon};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub url: Url,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryItem {
    pub url: Url,
    pub title: Option<String>,
    pub visited_ms: u64,
}

pub(crate) fn suggestions_popup(
    ctx: &egui::Context,
    anchor: Rect,
    items: &[Suggestion],
    selected: &mut Option<usize>,
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let (down, up) = ctx.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp)));
    if down {
        *selected = Some(selected.map_or(0, |s| (s + 1).min(items.len() - 1)));
    }
    if up {
        *selected = selected.and_then(|s| s.checked_sub(1));
    }
    let mut chosen = None;
    egui::Area::new(egui::Id::new("klin-url-suggestions"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + Vec2::new(0.0, 2.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.set_width(anchor.width());
                    for (i, item) in items.iter().enumerate() {
                        let title = if item.title.trim().is_empty() {
                            item.url.to_string()
                        } else {
                            item.title.clone()
                        };
                        let text = format!("{title}  —  {}", crate::display_url(&item.url));
                        let response = ui.add(
                            egui::Button::selectable(*selected == Some(i), RichText::new(text))
                                .truncate()
                                .min_size(Vec2::new(ui.available_width(), 26.0)),
                        );
                        if response.hovered() && ctx.input(|i| i.pointer.delta() != Vec2::ZERO) {
                            *selected = Some(i);
                        }
                        if response.clicked() {
                            chosen = Some(i);
                        }
                    }
                });
        });
    chosen
}

pub(crate) fn bookmark_panel(
    ctx: &egui::Context,
    anchor: Rect,
    url: &Url,
    title: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let width = 320.0;
    let pos = egui::pos2((anchor.right() - width).max(4.0), anchor.bottom() + 4.0);
    let mut open = true;
    let area = egui::Area::new(egui::Id::new("klin-bookmark-panel"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(width);

                    ui.label(RichText::new("Закладка добавлена").strong());
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(if title.is_empty() {
                            url.as_str()
                        } else {
                            title
                        })
                        .truncate(),
                    );
                    ui.label(RichText::new(crate::display_url(url)).small().weak());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Удалить закладку").clicked() {
                            actions.push(Action::RemoveBookmark(url.clone()));
                            open = false;
                        }
                        if ui.button("Готово").clicked() {
                            open = false;
                        }
                    });
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

pub(crate) fn star_button(ui: &mut egui::Ui, enabled: bool, bookmarked: bool) -> egui::Response {
    let response = icons::button(
        ui,
        enabled,
        if bookmarked {
            Icon::StarFilled
        } else {
            Icon::Star
        },
        30.0,
    );
    let hint = if bookmarked {
        "Изменить закладку (Ctrl+D)"
    } else {
        "Добавить страницу в закладки (Ctrl+D)"
    };
    response.on_hover_text(hint)
}

fn group_name(visited_ms: u64, now_ms: u64) -> &'static str {
    const DAY: u64 = 24 * 60 * 60 * 1000;

    let today_start = now_ms - now_ms % DAY;
    if visited_ms >= today_start {
        "Сегодня"
    } else if visited_ms >= today_start - DAY {
        "Вчера"
    } else if visited_ms >= today_start - 7 * DAY {
        "Последние 7 дней"
    } else if visited_ms >= today_start - 31 * DAY {
        "Этот месяц"
    } else {
        "Старше"
    }
}

pub(crate) fn history_sidebar(
    ui: &mut egui::Ui,
    items: &[HistoryItem],
    filter: &mut String,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("История").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, true, Icon::Close, 22.0)
                .on_hover_text("Закрыть боковую панель")
                .clicked()
            {
                actions.push(Action::ToggleHistorySidebar);
            }
        });
    });
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Поиск по истории")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let needle = filter.to_lowercase();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut group = "";
            for item in items {
                let title = item
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| item.url.to_string());
                if !needle.is_empty()
                    && !title.to_lowercase().contains(&needle)
                    && !item.url.as_str().to_lowercase().contains(&needle)
                {
                    continue;
                }
                let g = group_name(item.visited_ms, now_ms);
                if g != group {
                    group = g;
                    ui.add_space(4.0);
                    ui.label(RichText::new(g).small().weak());
                }
                let response = ui
                    .add(
                        egui::Button::new(title)
                            .frame(false)
                            .truncate()
                            .min_size(Vec2::new(ui.available_width(), 22.0)),
                    )
                    .on_hover_text(item.url.as_str());
                if response.clicked() {
                    actions.push(Action::OpenUrl(item.url.clone()));
                }
            }
            if items.is_empty() {
                ui.label(RichText::new("История пуста").weak());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_groups() {
        let day = 24 * 60 * 60 * 1000;
        let now = 100 * day + 5 * 60 * 60 * 1000;
        assert_eq!(group_name(now - 1000, now), "Сегодня");
        assert_eq!(group_name(now - day, now), "Вчера");
        assert_eq!(group_name(now - 3 * day, now), "Последние 7 дней");
        assert_eq!(group_name(now - 20 * day, now), "Этот месяц");
        assert_eq!(group_name(now - 90 * day, now), "Старше");
    }
}
