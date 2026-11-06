use crate::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedVisit {
    pub at_ms: u64,
    pub visit_type: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedPlace {
    pub url: Url,
    pub title: Option<String>,
    pub visits: Vec<ImportedVisit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkRoot {
    Menu,
    Toolbar,
    Unfiled,
    Mobile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportedBookmark {
    Bookmark {
        url: Url,
        title: Option<String>,
        added_ms: u64,
    },
    Folder {
        title: String,
        added_ms: u64,
        children: Vec<ImportedBookmark>,
    },
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedRoot {
    pub root: BookmarkRoot,
    pub children: Vec<ImportedBookmark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedCookie {
    pub name: String,
    pub value: String,
    pub host: String,
    pub path: String,
    pub expires_ms: Option<u64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportedData {
    pub places: Vec<ImportedPlace>,
    pub bookmarks: Vec<ImportedRoot>,
    pub cookies: Vec<ImportedCookie>,
}
