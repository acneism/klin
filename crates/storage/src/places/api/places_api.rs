/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::places::db::db::{PlacesDb, SharedPlacesDb};
use crate::places::error::*;
use crate::support::interrupt::register_interrupt;
use crate::support::sql::path::normalize_database_path;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use rusqlite::OpenFlags;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ConnectionType {
    ReadOnly = 1,
    ReadWrite = 2,
    Sync = 3,
}

impl ConnectionType {
    pub fn from_primitive(p: u8) -> Option<Self> {
        match p {
            1 => Some(ConnectionType::ReadOnly),
            2 => Some(ConnectionType::ReadWrite),
            3 => Some(ConnectionType::Sync),
            _ => None,
        }
    }
}

impl ConnectionType {
    pub fn rusqlite_flags(self) -> OpenFlags {
        let common_flags = OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        match self {
            ConnectionType::ReadOnly => common_flags | OpenFlags::SQLITE_OPEN_READ_ONLY,
            ConnectionType::ReadWrite => {
                common_flags | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE
            }
            ConnectionType::Sync => common_flags | OpenFlags::SQLITE_OPEN_READ_WRITE,
        }
    }
}

lazy_static! {
    static ref APIS: Mutex<HashMap<PathBuf, Weak<PlacesApi>>> = Mutex::new(HashMap::new());
}

static ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct PlacesApi {
    db_name: PathBuf,
    write_connection: Mutex<Option<PlacesDb>>,
    coop_tx_lock: Arc<Mutex<()>>,
    sync_connection: Mutex<Weak<SharedPlacesDb>>,
    id: usize,
}

impl PlacesApi {
    pub fn new(db_name: impl AsRef<Path>) -> Result<Arc<Self>> {
        let db_name = normalize_database_path(db_name)?;
        Self::new_or_existing(db_name)
    }

    pub fn new_memory(db_name: &str) -> Result<Arc<Self>> {
        let name = PathBuf::from(format!("file:{}?mode=memory&cache=shared", db_name));
        Self::new_or_existing(name)
    }
    fn new_or_existing_into(
        target: &mut HashMap<PathBuf, Weak<PlacesApi>>,
        db_name: PathBuf,
    ) -> Result<Arc<Self>> {
        let id = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        match target.get(&db_name).and_then(Weak::upgrade) {
            Some(existing) => Ok(existing),
            None => {
                let coop_tx_lock = Arc::new(Mutex::new(()));
                let connection = PlacesDb::open(
                    &db_name,
                    ConnectionType::ReadWrite,
                    id,
                    coop_tx_lock.clone(),
                )?;
                let new = PlacesApi {
                    db_name: db_name.clone(),
                    write_connection: Mutex::new(Some(connection)),
                    sync_connection: Mutex::new(Weak::new()),
                    id,
                    coop_tx_lock,
                };
                let arc = Arc::new(new);
                target.insert(db_name, Arc::downgrade(&arc));
                Ok(arc)
            }
        }
    }

    fn new_or_existing(db_name: PathBuf) -> Result<Arc<Self>> {
        let mut guard = APIS.lock();
        Self::new_or_existing_into(&mut guard, db_name)
    }

    pub fn open_connection(&self, conn_type: ConnectionType) -> Result<PlacesDb> {
        match conn_type {
            ConnectionType::ReadOnly => PlacesDb::open(
                self.db_name.clone(),
                ConnectionType::ReadOnly,
                self.id,
                self.coop_tx_lock.clone(),
            ),
            ConnectionType::ReadWrite => {
                let mut guard = self.write_connection.lock();
                match guard.take() {
                    None => Err(Error::ConnectionAlreadyOpen),
                    Some(db) => Ok(db),
                }
            }
            ConnectionType::Sync => {
                panic!("Use `get_sync_connection` to open a sync connection");
            }
        }
    }

    pub fn get_sync_connection(&self) -> Result<Arc<SharedPlacesDb>> {
        let mut conn = self.sync_connection.lock();
        match conn.upgrade() {
            Some(db) => Ok(db),
            None => {
                let db = Arc::new(SharedPlacesDb::new(PlacesDb::open(
                    self.db_name.clone(),
                    ConnectionType::Sync,
                    self.id,
                    self.coop_tx_lock.clone(),
                )?));
                register_interrupt(Arc::<SharedPlacesDb>::downgrade(&db));

                *conn = Arc::downgrade(&db);
                Ok(db)
            }
        }
    }

    pub fn close_connection(&self, connection: PlacesDb) -> Result<()> {
        if connection.api_id() != self.id {
            return Err(Error::WrongApiForClose);
        }
        if connection.conn_type() == ConnectionType::ReadWrite {
            let mut guard = self.write_connection.lock();
            assert!((*guard).is_none());
            *guard = Some(connection);
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ATOMIC_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub fn new_mem_api() -> Arc<PlacesApi> {
        crate::support::error::init_for_tests();

        let counter = ATOMIC_COUNTER.fetch_add(1, Ordering::Relaxed);
        PlacesApi::new_memory(&format!("test-api-{}", counter)).expect("should get an API")
    }

    pub fn new_mem_connection() -> PlacesDb {
        new_mem_api()
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get a connection")
    }

    pub struct MemConnections {
        pub read: PlacesDb,
        pub write: PlacesDb,
        pub api: Arc<PlacesApi>,
    }

    pub fn new_mem_connections() -> MemConnections {
        let api = new_mem_api();
        let read = api
            .open_connection(ConnectionType::ReadOnly)
            .expect("should get a read connection");
        let write = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get a write connection");
        MemConnections { read, write, api }
    }
}

#[cfg(test)]
mod tests {
    use super::test::*;
    use super::*;
    use crate::support::sql::ConnExt;

    #[test]
    fn test_multi_writers_fails() {
        let api = new_mem_api();
        let writer1 = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer");
        api.open_connection(ConnectionType::ReadWrite)
            .expect_err("should fail to get second writer");

        api.close_connection(writer1)
            .expect("should be able to close");
        api.open_connection(ConnectionType::ReadWrite)
            .expect("should get a writer after closing the other");
    }

    #[test]
    fn test_shared_memory() {
        let api = new_mem_api();
        let writer = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer");
        writer
            .execute_batch(
                "CREATE TABLE test_table (test_value INTEGER);
                              INSERT INTO test_table VALUES (999)",
            )
            .expect("should insert");
        let reader = api
            .open_connection(ConnectionType::ReadOnly)
            .expect("should get reader");
        let val = reader
            .conn_ext_query_one::<i64>("SELECT test_value FROM test_table")
            .expect("should get value");
        assert_eq!(val, 999);
    }

    #[test]
    fn test_reader_before_writer() {
        let api = new_mem_api();
        let reader = api
            .open_connection(ConnectionType::ReadOnly)
            .expect("should get reader");
        let writer = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer");
        writer
            .execute_batch(
                "CREATE TABLE test_table (test_value INTEGER);
                              INSERT INTO test_table VALUES (999)",
            )
            .expect("should insert");
        let val = reader
            .conn_ext_query_one::<i64>("SELECT test_value FROM test_table")
            .expect("should get value");
        assert_eq!(val, 999);
    }

    #[test]
    fn test_wrong_writer_close() {
        let api = new_mem_api();

        let _writer = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer");

        let fake_api = new_mem_api();
        let fake_writer = fake_api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer 2");

        assert!(matches!(
            api.close_connection(fake_writer).unwrap_err(),
            Error::WrongApiForClose
        ));
    }

    #[test]
    fn test_valid_writer_close() {
        let api = new_mem_api();
        let writer = api
            .open_connection(ConnectionType::ReadWrite)
            .expect("should get writer");

        api.close_connection(writer)
            .expect("Should allow closing own connection");

        assert!(api.open_connection(ConnectionType::ReadWrite).is_ok());
    }
}
