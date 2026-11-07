use egui::{RichText, Vec2};
use klin_shared::bookmarks::BookmarkNode;

use crate::Action;

const ITEM_MAX_CHARS: usize = 24;

fn short(title: &str) -> String {
    if title.chars().count() <= ITEM_MAX_CHARS {
        return title.to_owned();
    }
    let mut out: String = title.chars().take(ITEM_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

pub(crate) fn visible(setting: &str, url: Option<&klin_shared::Url>) -> bool {
    match setting {
        "always" => true,
        "never" => false,
        _ => {
            url.is_none_or(|u| matches!(u.as_str(), "about:newtab" | "about:home" | "about:blank"))
        }
    }
}

pub(crate) fn show(ui: &mut egui::Ui, items: &[BookmarkNode], actions: &mut Vec<Action>) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if items.is_empty() {
            ui.label(
                RichText::new(
                    "Чтобы быстро открывать закладки, размещайте их здесь, на панели закладок.",
                )
                .small()
                .weak(),
            );
        }
        for item in items {
            match item {
                BookmarkNode::Bookmark { url, title } => {
                    let response = ui
                        .add(egui::Button::new(RichText::new(short(title))).frame(false))
                        .on_hover_text(format!("{title}\n{url}"));
                    if response.clicked() {
                        actions.push(Action::OpenBookmark {
                            url: url.clone(),
                            new_tab: false,
                        });
                    }
                    if response.middle_clicked() {
                        actions.push(Action::OpenBookmark {
                            url: url.clone(),
                            new_tab: true,
                        });
                    }
                }
                BookmarkNode::Folder { title, children } => {
                    ui.menu_button(RichText::new(format!("{} ▾", short(title))), |ui| {
                        folder_menu(ui, children, actions);
                    });
                }
                BookmarkNode::Separator => {
                    ui.separator();
                }
            }
        }
    });
    ui.add_space(2.0);
}

fn folder_menu(ui: &mut egui::Ui, items: &[BookmarkNode], actions: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    if items.is_empty() {
        ui.label(RichText::new("(Пусто)").weak());
    }
    for item in items {
        match item {
            BookmarkNode::Bookmark { url, title } => {
                let response = ui
                    .add(
                        egui::Button::new(title)
                            .truncate()
                            .min_size(Vec2::new(ui.available_width(), 0.0)),
                    )
                    .on_hover_text(url.as_str());
                if response.clicked() {
                    actions.push(Action::OpenBookmark {
                        url: url.clone(),
                        new_tab: false,
                    });
                    ui.close();
                }
                if response.middle_clicked() {
                    actions.push(Action::OpenBookmark {
                        url: url.clone(),
                        new_tab: true,
                    });
                    ui.close();
                }
            }
            BookmarkNode::Folder { title, children } => {
                ui.menu_button(title, |ui| folder_menu(ui, children, actions));
            }
            BookmarkNode::Separator => {
                ui.separator();
            }
        }
    }
}
