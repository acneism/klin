use crate::input::{Key, Modifiers, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    NewWindow,
    NewPrivateWindow,
    CloseWindow,
    NewTab,
    CloseTab,
    UndoCloseTab,
    NextTab,
    PreviousTab,
    SelectTab(u8),
    SelectLastTab,
    FocusUrlBar,
    Back,
    Forward,
    Reload,
    ReloadSkipCache,
    Stop,
    Home,
    Find,
    FindAgain,
    FindPrevious,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Bookmark,
    ToggleBookmarksToolbar,
    ShowAllBookmarks,
    ToggleHistorySidebar,
    ShowAllHistory,
    ShowDownloads,
    ToggleReaderMode,
    ToggleFullScreen,
    Quit,
}

impl Command {
    pub fn firefox_id(self) -> Option<&'static str> {
        use Command::*;
        Some(match self {
            NewWindow => "cmd_newNavigator",
            NewPrivateWindow => "Tools:PrivateBrowsing",
            CloseWindow => "cmd_closeWindow",
            NewTab => "cmd_newNavigatorTab",
            CloseTab => "cmd_close",
            UndoCloseTab => "History:UndoCloseTab",
            FocusUrlBar => "Browser:OpenLocation",
            Back => "Browser:Back",
            Forward => "Browser:Forward",
            Reload => "Browser:Reload",
            ReloadSkipCache => "Browser:ReloadSkipCache",
            Stop => "Browser:Stop",
            Home => "Browser:Home",
            Find => "cmd_find",
            FindAgain => "cmd_findAgain",
            FindPrevious => "cmd_findPrevious",
            ZoomIn => "cmd_fullZoomEnlarge",
            ZoomOut => "cmd_fullZoomReduce",
            ZoomReset => "cmd_fullZoomReset",
            Bookmark => "Browser:AddBookmarkAs",
            ShowAllBookmarks => "Browser:ShowAllBookmarks",
            ShowAllHistory => "Browser:ShowAllHistory",
            ShowDownloads => "Tools:Downloads",
            ToggleReaderMode => "View:ReaderView",
            ToggleFullScreen => "View:FullScreen",
            Quit => "cmd_quitApplication",
            NextTab
            | PreviousTab
            | SelectTab(_)
            | SelectLastTab
            | ToggleBookmarksToolbar
            | ToggleHistorySidebar => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl KeyChord {
    pub const fn new(modifiers: Modifiers, key: Key) -> Self {
        Self { modifiers, key }
    }
}

const fn ch(c: char) -> Key {
    Key::Char(c)
}

const fn named(k: NamedKey) -> Key {
    Key::Named(k)
}

pub fn default_shortcuts() -> Vec<(KeyChord, Command)> {
    use Command::*;
    use Modifiers as M;
    use NamedKey::*;
    let mut table = vec![
        (KeyChord::new(M::CTRL, ch('n')), NewWindow),
        (KeyChord::new(M::CTRL_SHIFT, ch('p')), NewPrivateWindow),
        (KeyChord::new(M::CTRL_SHIFT, ch('w')), CloseWindow),
        (KeyChord::new(M::CTRL, ch('t')), NewTab),
        (KeyChord::new(M::CTRL, ch('w')), CloseTab),
        (KeyChord::new(M::CTRL, named(F(4))), CloseTab),
        (KeyChord::new(M::CTRL_SHIFT, ch('t')), UndoCloseTab),
        (KeyChord::new(M::CTRL, named(Tab)), NextTab),
        (KeyChord::new(M::CTRL, named(PageDown)), NextTab),
        (KeyChord::new(M::CTRL_SHIFT, named(Tab)), PreviousTab),
        (KeyChord::new(M::CTRL, named(PageUp)), PreviousTab),
        (KeyChord::new(M::CTRL, ch('9')), SelectLastTab),
        (KeyChord::new(M::CTRL, ch('l')), FocusUrlBar),
        (KeyChord::new(M::ALT, ch('d')), FocusUrlBar),
        (KeyChord::new(M::NONE, named(F(6))), FocusUrlBar),
        (KeyChord::new(M::ALT, named(ArrowLeft)), Back),
        (KeyChord::new(M::ALT, named(ArrowRight)), Forward),
        (KeyChord::new(M::NONE, named(F(5))), Reload),
        (KeyChord::new(M::CTRL, ch('r')), Reload),
        (KeyChord::new(M::CTRL, named(F(5))), ReloadSkipCache),
        (KeyChord::new(M::CTRL_SHIFT, ch('r')), ReloadSkipCache),
        (KeyChord::new(M::NONE, named(Escape)), Stop),
        (KeyChord::new(M::ALT, named(NamedKey::Home)), Command::Home),
        (KeyChord::new(M::CTRL, ch('f')), Find),
        (KeyChord::new(M::NONE, named(F(3))), FindAgain),
        (KeyChord::new(M::CTRL, ch('g')), FindAgain),
        (KeyChord::new(M::SHIFT, named(F(3))), FindPrevious),
        (KeyChord::new(M::CTRL_SHIFT, ch('g')), FindPrevious),
        (KeyChord::new(M::CTRL, ch('=')), ZoomIn),
        (KeyChord::new(M::CTRL, ch('+')), ZoomIn),
        (KeyChord::new(M::CTRL, ch('-')), ZoomOut),
        (KeyChord::new(M::CTRL, ch('0')), ZoomReset),
        (KeyChord::new(M::CTRL, ch('d')), Bookmark),
        (
            KeyChord::new(M::CTRL_SHIFT, ch('b')),
            ToggleBookmarksToolbar,
        ),
        (KeyChord::new(M::CTRL_SHIFT, ch('o')), ShowAllBookmarks),
        (KeyChord::new(M::CTRL, ch('h')), ToggleHistorySidebar),
        (KeyChord::new(M::CTRL_SHIFT, ch('h')), ShowAllHistory),
        (KeyChord::new(M::CTRL, ch('j')), ShowDownloads),
        (KeyChord::new(M::CTRL_ALT, ch('r')), ToggleReaderMode),
        (KeyChord::new(M::NONE, named(F(9))), ToggleReaderMode),
        (KeyChord::new(M::NONE, named(F(11))), ToggleFullScreen),
        (KeyChord::new(M::CTRL_SHIFT, ch('q')), Quit),
    ];
    for n in 1..=8u8 {
        let digit = char::from(b'0' + n);
        table.push((KeyChord::new(M::CTRL, ch(digit)), SelectTab(n - 1)));
    }
    table
}

pub fn lookup(shortcuts: &[(KeyChord, Command)], chord: KeyChord) -> Option<Command> {
    shortcuts
        .iter()
        .find(|(c, _)| *c == chord)
        .map(|(_, cmd)| *cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_chords() {
        let table = default_shortcuts();
        let unique: HashSet<_> = table.iter().map(|(c, _)| *c).collect();
        assert_eq!(unique.len(), table.len());
    }

    #[test]
    fn firefox_core_shortcuts() {
        let t = default_shortcuts();
        let get = |m, k| lookup(&t, KeyChord::new(m, k));
        assert_eq!(get(Modifiers::CTRL, Key::Char('t')), Some(Command::NewTab));
        assert_eq!(
            get(Modifiers::CTRL_SHIFT, Key::Char('t')),
            Some(Command::UndoCloseTab)
        );
        assert_eq!(
            get(Modifiers::ALT, Key::Char('d')),
            Some(Command::FocusUrlBar)
        );
        assert_eq!(
            get(Modifiers::CTRL, Key::Char('3')),
            Some(Command::SelectTab(2))
        );
        assert_eq!(
            get(Modifiers::CTRL, Key::Named(NamedKey::F(5))),
            Some(Command::ReloadSkipCache)
        );
    }
}
