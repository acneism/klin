//! Снимок сессии: окна, вкладки, их история переходов и закрытые вкладки.
//! Ядро строит его из своей модели, хранилище пишет в `sessionstore.jsonlz4` (формат Firefox).

use crate::Url;

/// Запись истории переходов вкладки.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryState {
    pub url: Url,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabState {
    pub entries: Vec<EntryState>,
    /// Текущая запись, с нуля (в файле Firefox — с единицы).
    pub index: usize,
}

impl TabState {
    pub fn current(&self) -> Option<&EntryState> {
        self.entries.get(self.index).or_else(|| self.entries.last())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTabState {
    pub tab: TabState,
    /// Позиция вкладки в окне на момент закрытия.
    pub pos: usize,
    /// Время закрытия, мс с начала эпохи Unix.
    pub closed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowState {
    pub tabs: Vec<TabState>,
    /// Выбранная вкладка, с нуля.
    pub selected: usize,
    pub closed_tabs: Vec<ClosedTabState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub windows: Vec<WindowState>,
    /// Активное окно, с нуля.
    pub selected_window: usize,
}
