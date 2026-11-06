/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::places::error::{Error, Result};
use crate::support::guid::Guid as SyncGuid;

pub mod bookmarks {
    pub const LAST_SYNC_META_KEY: &str = "bookmarks_last_sync_time";
    pub const GLOBAL_SYNCID_META_KEY: &str = "bookmarks_global_sync_id";
    pub const COLLECTION_SYNCID_META_KEY: &str = "bookmarks_sync_id";
}

pub mod history {
    pub const LAST_SYNC_META_KEY: &str = "history_last_sync_time";
    pub const GLOBAL_SYNCID_META_KEY: &str = "history_global_sync_id";
    pub const COLLECTION_SYNCID_META_KEY: &str = "history_sync_id";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollSyncIds {
    pub global: SyncGuid,
    pub coll: SyncGuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineSyncAssociation {
    Disconnected,
    Connected(CollSyncIds),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum SyncedBookmarkKind {
    Bookmark = 1,
    Query = 2,
    Folder = 3,
    Livemark = 4,
    Separator = 5,
}

impl SyncedBookmarkKind {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            1 => Ok(Self::Bookmark),
            2 => Ok(Self::Query),
            3 => Ok(Self::Folder),
            4 => Ok(Self::Livemark),
            5 => Ok(Self::Separator),
            _ => Err(Error::UnsupportedSyncedBookmarkKind(v)),
        }
    }
}
