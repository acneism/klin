/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! История и закладки (`places.sqlite`) — перенос компонента places из
//! mozilla/application-services, коммит 96b6f69902d143d6257c176c6e49e8c21e6d314b
//! (`components/places`, MPL 2.0).
//!
//! Почему перенос, а не зависимость: places требует rusqlite 0.37, Servo 0.7.0 — rusqlite 0.38,
//! а две копии SQLite в одной программе Cargo не допускает. Изменения при переносе:
//! rusqlite 0.38; убраны синхронизация (`bookmark_sync`, `history_sync`) — вернётся с Firefox
//! Sync в 0.3, но колонки синхронизации в схеме сохранены; убраны uniffi/FFI, телеметрия glean,
//! импорт из iOS и `history_metadata` (функция мобильного Firefox).

#![allow(dead_code)]

pub mod api;
pub mod db;
pub mod error;
pub mod ffi_types;
pub mod frecency;
pub mod hash;
pub mod match_impl;
pub mod observation;
pub mod storage;
pub mod sync_meta;
#[cfg(test)]
mod tests;
pub mod types;
mod util;

pub use crate::places::api::apply_observation;
#[cfg(test)]
pub use crate::places::api::places_api::test;
pub use crate::places::api::places_api::{ConnectionType, PlacesApi};
pub use crate::places::db::PlacesDb;
pub use crate::places::error::*;
pub use crate::places::observation::*;
pub use crate::places::storage::{PageInfo, RowId};
pub use crate::places::types::*;
