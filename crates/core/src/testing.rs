//! Тестовая реализация [`Engine`] без Servo: логика вкладок, сессий и команд
//! проверяется за секунды и без сборки движка.

use std::collections::HashMap;

use klin_shared::{RequestId, TabId, Url, WindowId};

use crate::engine::{Engine, Reply};
use crate::input::InputEvent;

/// Вызов движка, записанный [`FakeEngine`].
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    OpenTab(TabId, WindowId, Url, bool),
    CloseTab(TabId),
    Load(TabId, Url),
    GoBack(TabId),
    GoForward(TabId),
    Reload(TabId, bool),
    Stop(TabId),
    SetZoom(TabId, f32),
    SetVisible(TabId, bool),
    SendInput(TabId, InputEvent),
    Answer(RequestId, Reply),
}

#[derive(Debug, Default)]
pub struct FakeEngine {
    pub calls: Vec<Call>,
    visible: HashMap<TabId, bool>,
}

impl FakeEngine {
    pub fn visible(&self, tab: TabId) -> bool {
        self.visible.get(&tab).copied().unwrap_or(false)
    }
}

impl Engine for FakeEngine {
    fn open_tab(&mut self, window: WindowId, url: &Url, private: bool) -> TabId {
        let tab = TabId::next();
        self.calls
            .push(Call::OpenTab(tab, window, url.clone(), private));
        tab
    }
    fn close_tab(&mut self, tab: TabId) {
        self.visible.remove(&tab);
        self.calls.push(Call::CloseTab(tab));
    }
    fn load(&mut self, tab: TabId, url: &Url) {
        self.calls.push(Call::Load(tab, url.clone()));
    }
    fn go_back(&mut self, tab: TabId) {
        self.calls.push(Call::GoBack(tab));
    }
    fn go_forward(&mut self, tab: TabId) {
        self.calls.push(Call::GoForward(tab));
    }
    fn reload(&mut self, tab: TabId, bypass_cache: bool) {
        self.calls.push(Call::Reload(tab, bypass_cache));
    }
    fn stop(&mut self, tab: TabId) {
        self.calls.push(Call::Stop(tab));
    }
    fn set_zoom(&mut self, tab: TabId, factor: f32) {
        self.calls.push(Call::SetZoom(tab, factor));
    }
    fn set_visible(&mut self, tab: TabId, visible: bool) {
        self.visible.insert(tab, visible);
        self.calls.push(Call::SetVisible(tab, visible));
    }
    fn send_input(&mut self, tab: TabId, input: InputEvent) {
        self.calls.push(Call::SendInput(tab, input));
    }
    fn answer(&mut self, request: RequestId, reply: Reply) {
        self.calls.push(Call::Answer(request, reply));
    }
}
