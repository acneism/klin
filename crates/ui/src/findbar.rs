//! Панель поиска на странице (Ctrl+F) — по образцу findbar Firefox: внизу окна, поиск по мере
//! ввода, Enter / Shift+Enter, F3, «С учётом регистра», «Только слова целиком», счётчик.

use std::collections::HashMap;

use egui::{Key, RichText};
use klin_core::browser::Tab;
use klin_core::{FindDirection, FindRequest, FindWrap};
use klin_shared::TabId;

use crate::Action;
use crate::icons::{self, Icon};

#[derive(Default)]
pub(crate) struct FindBar {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// Поставить фокус в поле и выделить запрос (Ctrl+F).
    pub focus: bool,
    /// Вернуть фокус в поле без выделения (после Enter).
    pub refocus: bool,
}

#[derive(Default)]
pub(crate) struct FindBars {
    bars: HashMap<TabId, FindBar>,
}

impl FindBars {
    pub fn open(&mut self, tab: TabId) {
        let bar = self.bars.entry(tab).or_default();
        bar.open = true;
        bar.focus = true;
    }

    /// F3 / Shift+F3: следующее совпадение; без запроса — открыть панель (как в Firefox).
    pub fn again(&mut self, tab: TabId, backwards: bool) -> Option<Action> {
        let bar = self.bars.entry(tab).or_default();
        if bar.query.is_empty() {
            bar.open = true;
            bar.focus = true;
            return None;
        }
        bar.open = true;
        let direction = if backwards {
            FindDirection::Previous
        } else {
            FindDirection::Next
        };
        Some(request(tab, bar, direction))
    }

    pub fn retain(&mut self, live: impl Fn(TabId) -> bool) {
        self.bars.retain(|t, _| live(*t));
    }

    pub fn get_mut(&mut self, tab: TabId) -> Option<&mut FindBar> {
        self.bars.get_mut(&tab).filter(|b| b.open)
    }
}

fn request(tab: TabId, bar: &FindBar, direction: FindDirection) -> Action {
    Action::Find {
        tab,
        request: FindRequest {
            query: bar.query.clone(),
            case_sensitive: bar.case_sensitive,
            whole_word: bar.whole_word,
            direction,
        },
    }
}

/// Строка состояния — тексты панели поиска Firefox.
pub(crate) fn status(tab: &Tab, query: &str) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let result = tab.find_result?;
    if result.total == 0 {
        return Some("Фраза не найдена".to_owned());
    }
    let noun = matches_noun(result.total);
    let count = format!("{} из {} {noun}", result.current, result.total);
    Some(match result.wrapped {
        Some(FindWrap::FromTop) => {
            format!("{count} · Достигнут конец страницы, продолжено с начала")
        }
        Some(FindWrap::FromBottom) => {
            format!("{count} · Достигнуто начало страницы, продолжено с конца")
        }
        None => count,
    })
}

/// Склонение как в строке Firefox `findbar-found-matches` (ru): «1 из 21 совпадения», «1 из 5 совпадений».
fn matches_noun(total: usize) -> &'static str {
    if total % 10 == 1 && total % 100 != 11 {
        "совпадения"
    } else {
        "совпадений"
    }
}

/// Нарисовать панель. Возвращает `false`, если её закрыли.
pub(crate) fn show(
    ui: &mut egui::Ui,
    tab: &Tab,
    bar: &mut FindBar,
    actions: &mut Vec<Action>,
) -> bool {
    let mut open = true;
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        let id = egui::Id::new(("klin-find", tab.id));
        let field = ui.add(
            egui::TextEdit::singleline(&mut bar.query)
                .id(id)
                .hint_text("Найти на странице")
                .desired_width(240.0),
        );
        if bar.focus {
            bar.focus = false;
            field.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let range = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(bar.query.chars().count()),
                );
                state.cursor.set_char_range(Some(range));
                state.store(ui.ctx(), id);
            }
        }
        if bar.refocus {
            bar.refocus = false;
            field.request_focus();
        }
        if field.changed() {
            // Поиск по мере ввода, как в Firefox.
            actions.push(request(tab.id, bar, FindDirection::First));
        }
        if field.lost_focus() {
            let (enter, shift, escape) = ui.input(|i| {
                (
                    i.key_pressed(Key::Enter),
                    i.modifiers.shift,
                    i.key_pressed(Key::Escape),
                )
            });
            if enter {
                let dir = if shift {
                    FindDirection::Previous
                } else {
                    FindDirection::Next
                };
                actions.push(request(tab.id, bar, dir));
                bar.refocus = true;
            } else if escape {
                open = false;
            }
        }
        let has_query = !bar.query.is_empty();
        if icons::button(ui, has_query, Icon::Down, 26.0)
            .on_hover_text("Следующее совпадение")
            .clicked()
        {
            actions.push(request(tab.id, bar, FindDirection::Next));
        }
        if icons::button(ui, has_query, Icon::Up, 26.0)
            .on_hover_text("Предыдущее совпадение")
            .clicked()
        {
            actions.push(request(tab.id, bar, FindDirection::Previous));
        }
        // Отступление от Firefox: в Servo нет CSS Custom Highlight API, подсветка всех
        // совпадений потребовала бы менять DOM страницы.
        let mut highlight = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut highlight, "Подсветить все"))
            .on_disabled_hover_text("Движок Servo пока не умеет подсвечивать совпадения");
        if ui
            .checkbox(&mut bar.case_sensitive, "С учётом регистра")
            .changed()
            | ui.checkbox(&mut bar.whole_word, "Только слова целиком")
                .changed()
        {
            actions.push(request(tab.id, bar, FindDirection::First));
        }
        if let Some(text) = status(tab, &bar.query) {
            ui.label(RichText::new(text).small());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, true, Icon::Close, 24.0)
                .on_hover_text("Закрыть панель поиска")
                .clicked()
            {
                open = false;
            }
        });
    });
    ui.add_space(3.0);
    if !open {
        bar.open = false;
        actions.push(request(tab.id, bar, FindDirection::Clear));
        actions.push(Action::FocusPage);
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_plural_like_firefox() {
        assert_eq!(matches_noun(1), "совпадения");
        assert_eq!(matches_noun(21), "совпадения");
        assert_eq!(matches_noun(11), "совпадений");
        assert_eq!(matches_noun(5), "совпадений");
    }
}
