/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

mod coop_transaction;

use crate::places::api::places_api::ConnectionType;
use crate::places::error::*;
use crate::support::sql::{ConnExt, UncheckedTransaction};
use coop_transaction::ChunkedCoopTransaction;
use rusqlite::Connection;

pub struct PlacesTransaction<'conn>(PlacesTransactionRepr<'conn>);

enum PlacesTransactionRepr<'conn> {
    ChunkedWrite(ChunkedCoopTransaction<'conn>),
    UnchunkedWrite(UncheckedTransaction<'conn>),
    ReadOnly(UncheckedTransaction<'conn>),
}

impl PlacesTransaction<'_> {
    #[inline]
    pub fn should_commit(&self) -> bool {
        match &self.0 {
            PlacesTransactionRepr::ChunkedWrite(tx) => tx.should_commit(),
            _ => true,
        }
    }

    #[inline]
    pub fn maybe_commit(&mut self) -> Result<()> {
        if let PlacesTransactionRepr::ChunkedWrite(tx) = &mut self.0 {
            tx.maybe_commit()?;
        } else {
            crate::support::error::report_error!(
                "places-nonchunked-maybe-commit",
                "maybe_commit called on a non-chunked transaction"
            );
            if cfg!(debug_assertions) {
                panic!("maybe_commit called on a non-chunked transaction");
            }
        }
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        match self.0 {
            PlacesTransactionRepr::ChunkedWrite(t) => t.commit()?,
            PlacesTransactionRepr::UnchunkedWrite(t) => t.commit()?,
            PlacesTransactionRepr::ReadOnly(t) => t.commit()?,
        };
        Ok(())
    }

    pub fn rollback(self) -> Result<()> {
        match self.0 {
            PlacesTransactionRepr::ChunkedWrite(t) => t.rollback()?,
            PlacesTransactionRepr::UnchunkedWrite(t) => t.rollback()?,
            PlacesTransactionRepr::ReadOnly(t) => t.rollback()?,
        };
        Ok(())
    }
}

impl super::PlacesDb {
    pub fn begin_transaction(&self) -> Result<PlacesTransaction<'_>> {
        Ok(PlacesTransaction(match self.conn_type() {
            ConnectionType::Sync => {
                PlacesTransactionRepr::ChunkedWrite(self.chunked_coop_trransaction()?)
            }
            ConnectionType::ReadWrite => {
                PlacesTransactionRepr::UnchunkedWrite(self.coop_transaction()?)
            }
            ConnectionType::ReadOnly => {
                PlacesTransactionRepr::ReadOnly(self.unchecked_transaction()?)
            }
        }))
    }
}

impl std::ops::Deref for PlacesTransaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match &self.0 {
            PlacesTransactionRepr::ChunkedWrite(t) => t,
            PlacesTransactionRepr::UnchunkedWrite(t) => t,
            PlacesTransactionRepr::ReadOnly(t) => t,
        }
    }
}

impl ConnExt for PlacesTransaction<'_> {
    #[inline]
    fn conn(&self) -> &Connection {
        self
    }
}
