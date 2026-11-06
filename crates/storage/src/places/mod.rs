/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

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
