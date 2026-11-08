/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

#[macro_use]
mod error;
mod login;

mod db;
mod schema;
mod store;
mod sync_meta;
mod util;

pub use crate::logins::db::{LoginDb, LoginsDeletionMetrics};
pub use crate::logins::error::*;
pub use crate::logins::login::*;
pub use crate::logins::store::*;
pub use crate::support::crypto::{
    EncryptorDecryptor, KeyManager, ManagedEncryptorDecryptor, StaticKeyManager, check_canary,
    create_canary, create_key,
};

#[cfg(test)]
pub mod test_utils {
    use super::*;
    use serde::{Serialize, de::DeserializeOwned};
    use std::sync::Arc;

    lazy_static::lazy_static! {
        pub static ref TEST_ENCRYPTION_KEY: String = crate::support::crypto::create_key();
        pub static ref TEST_ENCDEC: Arc<ManagedEncryptorDecryptor> = Arc::new(ManagedEncryptorDecryptor::new(Arc::new(StaticKeyManager::new(TEST_ENCRYPTION_KEY.clone()))));
    }

    pub fn encrypt_struct<T: Serialize>(fields: &T) -> String {
        let string = serde_json::to_string(fields).unwrap();
        let cipherbytes = TEST_ENCDEC.encrypt(string.as_bytes().into()).unwrap();
        std::str::from_utf8(&cipherbytes).unwrap().to_owned()
    }
    pub fn decrypt_struct<T: DeserializeOwned>(ciphertext: String) -> T {
        let jsonbytes = TEST_ENCDEC.decrypt(ciphertext.as_bytes().into()).unwrap();
        serde_json::from_str(std::str::from_utf8(&jsonbytes).unwrap()).unwrap()
    }
}
