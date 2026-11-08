/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::ffi::OsString;
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) use crate::support::error::report_error;
pub use crate::support::error::{debug, error, info, trace, warn};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Database is closed")]
    DatabaseClosed,

    #[error("Malformed incoming record")]
    MalformedIncomingRecord,

    #[error("Invalid login: {0}")]
    InvalidLogin(#[from] InvalidLogin),

    #[error("The `sync_status` column in DB has an illegal value: {0}")]
    BadSyncStatus(u8),

    #[error("No record with guid exists (when one was required): {0:?}")]
    NoSuchRecord(String),

    #[error("The logins tables are not empty")]
    NonEmptyTable,

    #[error("encryption failed: {0:?}")]
    EncryptionFailed(String),

    #[error("decryption failed: {0:?}")]
    DecryptionFailed(String),

    #[error("Error parsing JSON data: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Error executing SQL: {0}")]
    SqlError(#[from] rusqlite::Error),

    #[error("Error parsing URL: {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("Invalid path: {0:?}")]
    InvalidPath(OsString),

    #[error("CryptoError({0})")]
    CryptoError(#[from] crate::support::crypto::CryptoError),

    #[error("{0}")]
    Interrupted(#[from] crate::support::interrupt::Interrupted),

    #[error("IOError: {0}")]
    IOError(#[from] std::io::Error),

    #[error("Migration Error: {0}")]
    MigrationError(String),

    #[error("IncompatibleVersion: {0}")]
    IncompatibleVersion(i64),
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidLogin {
    #[error("Origin is empty")]
    EmptyOrigin,
    #[error("Password is empty")]
    EmptyPassword,
    #[error("Login already exists")]
    DuplicateLogin,
    #[error("Both `formActionOrigin` and `httpRealm` are present")]
    BothTargets,
    #[error("Neither `formActionOrigin` or `httpRealm` are present")]
    NoTarget,

    #[error("Login has illegal origin: {reason}")]
    IllegalOrigin { reason: String },
    #[error("Login has illegal field: {field_info}")]
    IllegalFieldValue { field_info: String },
}
