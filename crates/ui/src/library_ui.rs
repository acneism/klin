use egui::{RichText, Vec2};
use klin_shared::bookmarks::BookmarkNode;
use klin_shared::download::Download;
use klin_shared::import::BookmarkRoot;

use crate::places_ui::HistoryItem;
use crate::{Action, downloads};

pub const LIBRARY_URL: &str = "about:library";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LibrarySection {
    #[default]
    History,
    Downloads,
    Bookmarks(BookmarkRoot),
}

#[derive(Default)]
pub(crate) struct LibraryState {
    pub section: LibrarySection,
    filter: String,
    pub requested: bool,
}

pub(crate) fn root_name(root: BookmarkRoot) -> &'static str {
    match root {
        BookmarkRoot::Toolbar => "Панель закладок",
        BookmarkRoot::Menu => "Меню закладок",
        BookmarkRoot::Unfiled => "Другие закладки",
        BookmarkRoot::Mobile => "Мобильные закладки",
    }
}

pub(crate) struct LibraryData<'a> {
    pub history: &'a [HistoryItem],
    pub bookmarks: &'a [(BookmarkRoot, Vec<BookmarkNode>)],
    pub downloads: &'a [Download],
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    data: LibraryData<'_>,
    actions: &mut Vec<Action>,
) {
    if !state.requested {
        state.requested = true;
        actions.push(Action::LoadLibrary);
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.add_space(8.0);
            let mut entry =
                |ui: &mut egui::Ui, section: LibrarySection, label: &str, indent: bool| {
                    let text = if indent {
                        format!("    {label}")
                    } else {
                        label.to_owned()
                    };
                    if ui
                        .add(
                            egui::Button::selectable(state.section == section, text)
                                .min_size(Vec2::new(190.0, 24.0)),
                        )
                        .clicked()
                    {
                        state.section = section;
                    }
                };
            entry(ui, LibrarySection::History, "История", false);
            entry(ui, LibrarySection::Downloads, "Загрузки", false);
            ui.add_space(4.0);
            ui.label(RichText::new("Все закладки").strong());
            for root in [
                BookmarkRoot::Toolbar,
                BookmarkRoot::Menu,
                BookmarkRoot::Unfiled,
                BookmarkRoot::Mobile,
            ] {
                entry(ui, LibrarySection::Bookmarks(root), root_name(root), true);
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.add_space(8.0);
            let hint = match state.section {
                LibrarySection::History => "Поиск по истории",
                LibrarySection::Downloads => "Поиск загрузок",
                LibrarySection::Bookmarks(_) => "Поиск в закладках",
            };
            ui.add(
                egui::TextEdit::singleline(&mut state.filter)
                    .hint_text(hint)
                    .desired_width(420.0),
            );
            ui.add_space(6.0);
            let filter = state.filter.to_lowercase();
            let matches = |title: &str, url: &str| {
                filter.is_empty()
                    || title.to_lowercase().contains(&filter)
                    || url.to_lowercase().contains(&filter)
            };
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match state.section {
                    LibrarySection::History => {
                        let mut seen = std::collections::HashSet::new();
                        for item in data.history {
                            if !seen.insert(item.url.as_str()) {
                                continue;
                            }
                            let title = item
                                .title
                                .clone()
                                .filter(|t| !t.trim().is_empty())
                                .unwrap_or_else(|| item.url.to_string());
                            if !matches(&title, item.url.as_str()) {
                                continue;
                            }
                            row(
                                ui,
                                &title,
                                item.url.as_str(),
                                || Action::OpenUrl(item.url.clone()),
                                || Action::DeletePage(item.url.clone()),
                                actions,
                            );
                        }
                        if data.history.is_empty() {
                            ui.label(RichText::new("История пуста").weak());
                        }
                    }
                    LibrarySection::Downloads => {
                        for d in data.downloads.iter().rev() {
                            if !matches(&d.file_name(), d.url.as_str()) {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(d.file_name()).strong());
                                ui.label(RichText::new(downloads::status_text(d)).weak());
                                if ui.button("Показать в папке").clicked() {
                                    actions.push(Action::ShowDownloadInFolder(d.path.clone()));
                                }
                            });
                        }
                        if data.downloads.is_empty() {
                            ui.label(RichText::new("Загрузок нет").weak());
                        }
                    }
                    LibrarySection::Bookmarks(root) => {
                        let nodes = data
                            .bookmarks
                            .iter()
                            .find(|(r, _)| *r == root)
                            .map(|(_, n)| n.as_slice())
                            .unwrap_or(&[]);
                        tree(ui, nodes, &matches, actions);
                        if nodes.is_empty() {
                            ui.label(RichText::new("Здесь пока нет закладок").weak());
                        }
                    }
                });
        });
    });
}

fn row(
    ui: &mut egui::Ui,
    title: &str,
    url: &str,
    open: impl FnOnce() -> Action,
    delete: impl FnOnce() -> Action,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        let response = ui
            .add(
                egui::Button::new(title)
                    .frame(false)
                    .truncate()
                    .min_size(Vec2::new(360.0, 22.0)),
            )
            .on_hover_text(url);
        if response.clicked() {
            actions.push(open());
            return;
        }
        ui.label(
            RichText::new(crate::url_display::shorten(url))
                .small()
                .weak(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Удалить").clicked() {
                actions.push(delete());
            }
        });
    });
}

fn tree(
    ui: &mut egui::Ui,
    nodes: &[BookmarkNode],
    matches: &dyn Fn(&str, &str) -> bool,
    actions: &mut Vec<Action>,
) {
    for node in nodes {
        match node {
            BookmarkNode::Bookmark { id, url, title } => {
                if matches(title, url.as_str()) {
                    row(
                        ui,
                        title,
                        url.as_str(),
                        || Action::OpenBookmark {
                            url: url.clone(),
                            new_tab: false,
                        },
                        || Action::DeleteBookmark(id.clone()),
                        actions,
                    );
                }
            }
            BookmarkNode::Folder {
                id,
                title,
                children,
            } => {
                egui::CollapsingHeader::new(RichText::new(title).strong())
                    .id_salt(id)
                    .default_open(true)
                    .show(ui, |ui| tree(ui, children, matches, actions));
            }
            BookmarkNode::Separator => {
                ui.separator();
            }
        }
    }
}
