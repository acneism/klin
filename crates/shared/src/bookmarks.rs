use crate::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarkNode {
    Bookmark {
        url: Url,
        title: String,
    },
    Folder {
        title: String,
        children: Vec<BookmarkNode>,
    },
    Separator,
}
