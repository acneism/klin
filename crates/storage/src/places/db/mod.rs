/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#[allow(clippy::module_inception)]
pub mod db;
mod schema;
mod tx;
pub use self::tx::PlacesTransaction;

pub use crate::places::db::db::{GlobalChangeCounterTracker, PlacesDb, SharedPlacesDb};
