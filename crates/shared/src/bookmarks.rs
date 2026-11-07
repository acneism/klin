use crate::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarkNode {
    Bookmark {
        id: String,
        url: Url,
        title: String,
    },
    Folder {
        id: String,
        title: String,
        children: Vec<BookmarkNode>,
    },
    Separator,
}
