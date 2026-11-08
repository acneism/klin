/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::logins::db::LoginDb;
use crate::logins::error::*;
use crate::logins::login::{
    BulkResultEntry, EncryptedLogin, Login, LoginCandidate, LoginEntry, LoginEntryWithMeta,
};
use crate::support::crypto::EncryptorDecryptor;
use crate::support::sql::maintenance::run_maintenance;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

pub enum LoginOrErrorMessage {
    Login,
    String,
}

fn map_bulk_result_entry(
    enc_login: Result<EncryptedLogin>,
    encdec: &dyn EncryptorDecryptor,
) -> BulkResultEntry {
    match enc_login {
        Ok(enc_login) => match enc_login.decrypt(encdec) {
            Ok(login) => BulkResultEntry::Success { login },
            Err(error) => {
                warn!(
                    "Login could not be decrypted. This indicates a fundamental problem with the encryption key."
                );
                BulkResultEntry::Error {
                    message: error.to_string(),
                }
            }
        },
        Err(error) => BulkResultEntry::Error {
            message: error.to_string(),
        },
    }
}

pub struct LoginStore {
    pub db: Mutex<Option<LoginDb>>,
}

impl LoginStore {
    pub fn new(path: impl AsRef<Path>, encdec: Arc<dyn EncryptorDecryptor>) -> Result<Self> {
        let db = Mutex::new(Some(LoginDb::open(path, encdec)?));
        Ok(Self { db })
    }

    pub fn new_from_db(db: LoginDb) -> Self {
        let db = Mutex::new(Some(db));
        Self { db }
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Self {
        let db = Mutex::new(Some(LoginDb::open_in_memory()));
        Self { db }
    }

    pub fn lock_db(&self) -> Result<parking_lot::MappedMutexGuard<'_, LoginDb>> {
        parking_lot::MutexGuard::try_map(self.db.lock(), |db| db.as_mut())
            .map_err(|_| Error::DatabaseClosed)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.lock_db()?.count_all()? == 0)
    }

    pub fn list(&self) -> Result<Vec<Login>> {
        let db = self.lock_db()?;
        db.get_all().and_then(|logins| {
            logins
                .into_iter()
                .map(|login| login.decrypt(db.encdec.as_ref()))
                .collect()
        })
    }

    pub fn list_candidates(&self) -> Result<Vec<LoginCandidate>> {
        Ok(self
            .lock_db()?
            .get_all()?
            .into_iter()
            .map(LoginCandidate::from)
            .collect())
    }

    pub fn list_candidates_by_origin(
        &self,
        origins: Vec<String>,
        domains: Vec<String>,
    ) -> Result<Vec<LoginCandidate>> {
        Ok(self
            .lock_db()?
            .get_by_origins_or_domains(&origins, &domains)?
            .into_iter()
            .map(LoginCandidate::from)
            .collect())
    }

    pub fn count(&self) -> Result<i64> {
        self.lock_db()?.count_all()
    }

    pub fn count_by_origin(&self, origin: &str) -> Result<i64> {
        self.lock_db()?.count_by_origin(origin)
    }

    pub fn count_by_form_action_origin(&self, form_action_origin: &str) -> Result<i64> {
        self.lock_db()?
            .count_by_form_action_origin(form_action_origin)
    }

    pub fn get(&self, id: &str) -> Result<Option<Login>> {
        let db = self.lock_db()?;
        match db.get_by_id(id) {
            Ok(result) => match result {
                Some(enc_login) => enc_login.decrypt(db.encdec.as_ref()).map(Some),
                None => Ok(None),
            },
            Err(err) => Err(err),
        }
    }

    pub fn get_many(&self, ids: Vec<String>) -> Result<Vec<Login>> {
        let db = self.lock_db()?;
        db.get_many(&ids)?
            .into_iter()
            .map(|login| login.decrypt(db.encdec.as_ref()))
            .collect()
    }

    pub fn get_by_base_domain(&self, base_domain: &str) -> Result<Vec<Login>> {
        let db = self.lock_db()?;
        db.get_by_base_domain(base_domain).and_then(|logins| {
            logins
                .into_iter()
                .map(|login| login.decrypt(db.encdec.as_ref()))
                .collect()
        })
    }

    pub fn has_logins_by_base_domain(&self, base_domain: &str) -> Result<bool> {
        self.lock_db()?
            .get_by_base_domain(base_domain)
            .map(|logins| !logins.is_empty())
    }

    pub fn find_login_to_update(&self, entry: LoginEntry) -> Result<Option<Login>> {
        let db = self.lock_db()?;
        db.find_login_to_update(entry)
    }

    pub fn touch(&self, id: &str) -> Result<()> {
        self.lock_db()?.touch(id)
    }

    pub fn are_potentially_vulnerable_passwords(&self, ids: Vec<String>) -> Result<Vec<String>> {
        let db = self.lock_db()?;
        let ids: Vec<&str> = ids.iter().map(|id| &**id).collect();
        db.are_potentially_vulnerable_passwords(&ids)
    }

    pub fn is_potentially_vulnerable_password(&self, id: &str) -> Result<bool> {
        let db = self.lock_db()?;
        db.is_potentially_vulnerable_password(id)
    }

    pub fn record_potentially_vulnerable_passwords(&self, passwords: Vec<String>) -> Result<()> {
        let db = self.lock_db()?;
        db.record_potentially_vulnerable_passwords(passwords)
    }

    pub fn reset_all_breaches(&self) -> Result<()> {
        self.lock_db()?.reset_all_breaches()
    }

    pub fn record_breach_alert_dismissal(&self, id: &str) -> Result<()> {
        self.lock_db()?.record_breach_alert_dismissal(id)
    }

    pub fn record_breach_alert_dismissal_time(&self, id: &str, timestamp: i64) -> Result<()> {
        self.lock_db()?
            .record_breach_alert_dismissal_time(id, timestamp)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        self.lock_db()?.delete(id)
    }

    pub fn delete_many(&self, ids: Vec<String>) -> Result<Vec<bool>> {
        let ids: Vec<&str> = ids.iter().map(|id| &**id).collect();
        self.lock_db()?.delete_many(ids)
    }

    pub fn delete_all(&self) -> Result<Vec<String>> {
        self.lock_db()?.delete_all()
    }

    pub fn delete_all_except_fxa(&self) -> Result<Vec<String>> {
        self.lock_db()?.delete_all_except_fxa()
    }

    pub fn wipe_local(&self) -> Result<()> {
        self.lock_db()?.wipe_local()?;
        Ok(())
    }

    pub fn wipe_local_except_fxa(&self) -> Result<()> {
        self.lock_db()?.wipe_local_except_fxa()?;
        Ok(())
    }

    pub fn update(&self, id: &str, entry: LoginEntry) -> Result<Login> {
        let db = self.lock_db()?;
        db.update(id, entry)
            .and_then(|enc_login| enc_login.decrypt(db.encdec.as_ref()))
    }

    pub fn add(&self, entry: LoginEntry) -> Result<Login> {
        let db = self.lock_db()?;
        db.add(entry)
            .and_then(|enc_login| enc_login.decrypt(db.encdec.as_ref()))
    }

    pub fn add_many(&self, entries: Vec<LoginEntry>) -> Result<Vec<BulkResultEntry>> {
        let db = self.lock_db()?;
        let encdec = db.encdec.as_ref();
        db.add_many(entries).map(|enc_logins| {
            enc_logins
                .into_iter()
                .map(|enc_login| map_bulk_result_entry(enc_login, encdec))
                .collect()
        })
    }

    pub fn add_with_meta(&self, entry_with_meta: LoginEntryWithMeta) -> Result<Login> {
        let db = self.lock_db()?;
        db.add_with_meta(entry_with_meta)
            .and_then(|enc_login| enc_login.decrypt(db.encdec.as_ref()))
    }

    pub fn add_many_with_meta(
        &self,
        entries_with_meta: Vec<LoginEntryWithMeta>,
    ) -> Result<Vec<BulkResultEntry>> {
        let db = self.lock_db()?;
        let encdec = db.encdec.as_ref();
        db.add_many_with_meta(entries_with_meta).map(|enc_logins| {
            enc_logins
                .into_iter()
                .map(|enc_login| map_bulk_result_entry(enc_login, encdec))
                .collect()
        })
    }

    pub fn add_or_update(&self, entry: LoginEntry) -> Result<Login> {
        let db = self.lock_db()?;
        db.add_or_update(entry)
            .and_then(|enc_login| enc_login.decrypt(db.encdec.as_ref()))
    }

    pub fn run_maintenance(&self, options: Option<RunMaintenanceOptions>) -> Result<()> {
        let conn = self.lock_db()?;
        let options = options.unwrap_or_default();
        run_maintenance(&conn)?;
        if options.delete_undecryptable_records_for_remote_replacement {
            conn.delete_undecryptable_records_for_remote_replacement()?;
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        if let Some(db) = self.db.lock().take() {
            let _ = db.shutdown();
        }
    }
}

pub struct RunMaintenanceOptions {
    pub delete_undecryptable_records_for_remote_replacement: bool,
}

impl Default for RunMaintenanceOptions {
    fn default() -> Self {
        Self {
            delete_undecryptable_records_for_remote_replacement: true,
        }
    }
}

#[cfg(not(feature = "keydb"))]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::logins::util;
    use crate::support::crypto::{KeyManager, ManagedEncryptorDecryptor, create_key};
    use std::cmp::Reverse;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    fn assert_logins_equiv(a: &LoginEntry, b: &Login) {
        assert_eq!(a.origin, b.origin);
        assert_eq!(a.form_action_origin, b.form_action_origin);
        assert_eq!(a.http_realm, b.http_realm);
        assert_eq!(a.username_field, b.username_field);
        assert_eq!(a.password_field, b.password_field);
        assert_eq!(b.username, a.username);
        assert_eq!(b.password, a.password);
    }

    #[test]
    fn test_general() {
        let store = LoginStore::new_in_memory();
        let list = store.list().expect("Grabbing Empty list to work");
        assert_eq!(list.len(), 0);
        let start_us = util::system_time_ms_i64(SystemTime::now());

        let a = LoginEntry {
            origin: "https://www.example.com".into(),
            form_action_origin: Some("https://www.example.com".into()),
            username_field: "user_input".into(),
            password_field: "pass_input".into(),
            username: "user".into(),
            password: "password".into(),
            ..Default::default()
        };

        let b = LoginEntry {
            origin: "https://www.example2.com".into(),
            http_realm: Some("Some String Here".into()),
            username: "asdf".into(),
            password: "fdsa".into(),
            ..Default::default()
        };
        let a_id = store.add(a.clone()).expect("added a").id;
        let b_id = store.add(b.clone()).expect("added b").id;

        let a_from_db = store
            .get(&a_id)
            .expect("Not to error getting a")
            .expect("a to exist");

        assert_logins_equiv(&a, &a_from_db);
        assert!(a_from_db.time_created >= start_us);
        assert!(a_from_db.time_password_changed >= start_us);
        assert!(a_from_db.time_last_used >= start_us);
        assert_eq!(a_from_db.times_used, 1);

        let b_from_db = store
            .get(&b_id)
            .expect("Not to error getting b")
            .expect("b to exist");

        assert_logins_equiv(&LoginEntry { ..b.clone() }, &b_from_db);
        assert!(b_from_db.time_created >= start_us);
        assert!(b_from_db.time_password_changed >= start_us);
        assert!(b_from_db.time_last_used >= start_us);
        assert_eq!(b_from_db.times_used, 1);

        let mut list = store.list().expect("Grabbing list to work");
        assert_eq!(list.len(), 2);

        let mut expect = vec![a_from_db, b_from_db.clone()];

        list.sort_by_key(|b| Reverse(b.guid()));
        expect.sort_by_key(|b| Reverse(b.guid()));
        assert_eq!(list, expect);

        store.delete(&a_id).expect("Successful delete");
        assert!(
            store
                .get(&a_id)
                .expect("get after delete should still work")
                .is_none()
        );

        let list = store.list().expect("Grabbing list to work");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], b_from_db);

        let has_logins = store
            .has_logins_by_base_domain("example2.com")
            .expect("Expect a result for this origin");
        assert!(has_logins);

        let list = store
            .get_by_base_domain("example2.com")
            .expect("Expect a list for this origin");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], b_from_db);

        let has_logins = store
            .has_logins_by_base_domain("www.example.com")
            .expect("Expect a result for this origin");
        assert!(!has_logins);

        let list = store
            .get_by_base_domain("www.example.com")
            .expect("Expect an empty list");
        assert_eq!(list.len(), 0);

        let now_us = util::system_time_ms_i64(SystemTime::now());
        let b2 = LoginEntry {
            username: b.username.to_owned(),
            password: "newpass".into(),
            ..b
        };

        store
            .update(&b_id, b2.clone())
            .expect("update b should work");

        let b_after_update = store
            .get(&b_id)
            .expect("Not to error getting b")
            .expect("b to exist");

        assert_logins_equiv(&b2, &b_after_update);
        assert!(b_after_update.time_created >= start_us);
        assert!(b_after_update.time_created <= now_us);
        assert!(b_after_update.time_password_changed >= now_us);

        assert_eq!(b_after_update.time_last_used, b_from_db.time_last_used);
        assert_eq!(b_after_update.times_used, 1);
    }

    #[test]
    fn test_wipe_local_on_a_fresh_database_is_a_noop() {
        let db = LoginDb::open_in_memory();
        db.add_or_update(LoginEntry {
            origin: "https://www.example.com".into(),
            form_action_origin: Some("https://www.example.com".into()),
            username_field: "user_input".into(),
            password_field: "pass_input".into(),
            username: "coolperson21".into(),
            password: "p4ssw0rd".into(),
            ..Default::default()
        })
        .unwrap();
        assert!(db.wipe_local().unwrap() > 0);

        let db = LoginDb::open_in_memory();
        assert_eq!(db.wipe_local().unwrap(), 0);
    }

    #[test]
    fn test_shutdown() {
        let store = LoginStore::new_in_memory();
        store.shutdown();
        assert!(matches!(store.list(), Err(Error::DatabaseClosed)));
        assert!(store.db.lock().is_none());
    }

    struct CountingKeyManager {
        key: String,
        calls: AtomicUsize,
    }

    impl KeyManager for CountingKeyManager {
        fn get_key(&self) -> crate::support::crypto::CryptoResult<Vec<u8>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.key.as_bytes().into())
        }
    }

    fn store_with_encdec(encdec: Arc<dyn EncryptorDecryptor>) -> LoginStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        LoginStore::new_from_db(LoginDb::with_connection(conn, encdec).unwrap())
    }

    fn test_entry(origin: &str, username: &str) -> LoginEntry {
        LoginEntry {
            origin: origin.into(),
            http_realm: Some("Some Realm".into()),
            username: username.into(),
            password: "p4ssw0rd".into(),
            ..Default::default()
        }
    }

    #[test]
    fn test_list_candidates_does_not_need_the_key() {
        let key_manager = Arc::new(CountingKeyManager {
            key: create_key(),
            calls: AtomicUsize::new(0),
        });
        let store = store_with_encdec(Arc::new(ManagedEncryptorDecryptor::new(
            key_manager.clone(),
        )));

        let a = store
            .add(test_entry("https://www.a.com", "a-user"))
            .unwrap();
        let b = store
            .add(test_entry("https://www.b.com", "b-user"))
            .unwrap();

        assert!(key_manager.calls.load(Ordering::SeqCst) > 0);
        key_manager.calls.store(0, Ordering::SeqCst);

        let mut candidates = store.list_candidates().unwrap();
        assert_eq!(key_manager.calls.load(Ordering::SeqCst), 0);

        candidates.sort_by(|l, r| l.origin.cmp(&r.origin));
        assert_eq!(candidates.len(), 2);
        for (candidate, login) in candidates.iter().zip([&a, &b]) {
            assert_eq!(candidate.id, login.id);
            assert_eq!(candidate.origin, login.origin);
            assert_eq!(candidate.http_realm, login.http_realm);
            assert_eq!(candidate.form_action_origin, login.form_action_origin);
            assert_eq!(candidate.username_field, login.username_field);
            assert_eq!(candidate.password_field, login.password_field);
            assert_eq!(candidate.times_used, login.times_used);
            assert_eq!(candidate.time_created, login.time_created);
            assert_eq!(candidate.time_last_used, login.time_last_used);
            assert_eq!(candidate.time_password_changed, login.time_password_changed);
            assert_eq!(
                candidate.time_last_breach_alert_dismissed,
                login.time_last_breach_alert_dismissed
            );
        }
    }

    #[test]
    fn test_list_candidates_by_origin() {
        let key_manager = Arc::new(CountingKeyManager {
            key: create_key(),
            calls: AtomicUsize::new(0),
        });
        let store = store_with_encdec(Arc::new(ManagedEncryptorDecryptor::new(
            key_manager.clone(),
        )));

        let a = store
            .add(test_entry("https://www.a.com", "a-user"))
            .unwrap();
        let sub_a = store
            .add(test_entry("https://login.sub.a.com", "sub-a-user"))
            .unwrap();
        let b = store.add(test_entry("http://b.com", "b-user")).unwrap();
        store
            .add(test_entry("https://www.c.com", "c-user"))
            .unwrap();
        key_manager.calls.store(0, Ordering::SeqCst);

        let mut ids: Vec<String> = store
            .list_candidates_by_origin(vec!["http://b.com".into()], vec!["a.com".into()])
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        ids.sort_unstable();
        let mut expected = vec![a.id, sub_a.id, b.id];
        expected.sort_unstable();
        assert_eq!(ids, expected);

        assert_eq!(key_manager.calls.load(Ordering::SeqCst), 0);

        assert!(
            store
                .list_candidates_by_origin(vec![], vec![])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_get_many() {
        let store = LoginStore::new_in_memory();
        let a = store
            .add(test_entry("https://www.a.com", "a-user"))
            .unwrap();
        let b = store
            .add(test_entry("https://www.b.com", "b-user"))
            .unwrap();
        store
            .add(test_entry("https://www.c.com", "c-user"))
            .unwrap();

        assert_eq!(store.get_many(vec![]).unwrap(), vec![]);

        let mut got = store
            .get_many(vec![b.id.clone(), "no-such-guid".to_string(), a.id.clone()])
            .unwrap();
        got.sort_by(|l, r| l.origin.cmp(&r.origin));
        assert_eq!(got, vec![a, b]);
    }

    #[test]
    fn test_get_many_with_an_undecryptable_login() {
        let store = LoginStore::new_in_memory();
        let a = store
            .add(test_entry("https://www.a.com", "a-user"))
            .unwrap();
        let b = store
            .add(test_entry("https://www.b.com", "b-user"))
            .unwrap();

        store
            .lock_db()
            .unwrap()
            .db
            .execute(
                "UPDATE loginsL SET secFields = 'not-a-ciphertext' WHERE guid = ?",
                [&b.id],
            )
            .unwrap();

        assert!(store.get_many(vec![a.id.clone(), b.id]).is_err());
    }
}

#[test]
fn test_send() {
    fn ensure_send<T: Send>() {}
    ensure_send::<LoginStore>();
}
