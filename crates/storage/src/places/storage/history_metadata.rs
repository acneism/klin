/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::places::db::PlacesDb;
use crate::places::error::Result;
use crate::places::storage::RowId;
use crate::support::sql::ConnExt;

pub fn delete_between(db: &PlacesDb, start: i64, end: i64) -> Result<()> {
    db.execute_cached(
        "DELETE FROM moz_places_metadata
        WHERE updated_at > :start and updated_at < :end",
        &[(":start", &start), (":end", &end)],
    )?;
    Ok(())
}

pub fn delete_all_metadata_for_page(db: &PlacesDb, place_id: RowId) -> Result<()> {
    db.execute_cached(
        "DELETE FROM moz_places_metadata
         WHERE place_id = :place_id",
        &[(":place_id", &place_id)],
    )?;
    Ok(())
}
