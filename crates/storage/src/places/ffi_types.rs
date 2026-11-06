/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Типы данных из `ffi.rs` компонента places, которые используют хранилище и поиск
//! (сама обвязка uniffi для мобильных приложений не переносится).

use url::Url;

use crate::places::types::VisitType;
use crate::support::types::Timestamp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryVisitInfo {
    pub url: Url,
    pub title: Option<String>,
    pub timestamp: Timestamp,
    pub visit_type: VisitType,
    pub is_hidden: bool,
    pub preview_image_url: Option<Url>,
    pub is_remote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryVisitInfosWithBound {
    pub infos: Vec<HistoryVisitInfo>,
    pub bound: i64,
    pub offset: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopFrecentSiteInfo {
    pub url: Url,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub url: Url,
    pub title: String,
    pub frecency: i64,
}
