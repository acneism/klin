use std::path::PathBuf;

use crate::{DownloadId, Url};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    InProgress { received: u64, total: Option<u64> },
    Finished { size: u64 },
    Failed { error: String },
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub id: DownloadId,
    pub url: Url,
    pub path: PathBuf,
    pub state: DownloadState,
}

impl Download {
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.url.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadEvent {
    Started {
        id: DownloadId,
        url: Url,
        path: PathBuf,
        total: Option<u64>,
    },
    Progress {
        id: DownloadId,
        received: u64,
        total: Option<u64>,
    },
    Finished {
        id: DownloadId,
        size: u64,
    },
    Failed {
        id: DownloadId,
        error: String,
    },
    Canceled {
        id: DownloadId,
    },
}
