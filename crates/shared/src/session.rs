use crate::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryState {
    pub url: Url,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabState {
    pub entries: Vec<EntryState>,
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
    pub pos: usize,
    pub closed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowState {
    pub tabs: Vec<TabState>,
    pub selected: usize,
    pub closed_tabs: Vec<ClosedTabState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub windows: Vec<WindowState>,
    pub selected_window: usize,
}
