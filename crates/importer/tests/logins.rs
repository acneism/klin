use std::path::Path;

use klin_importer::nss::{NssError, logins_key};
use klin_importer::read_logins;

fn data(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn imports_logins_encrypted_by_nss() {
    let dir = data("nss-no-password");
    let logins = read_logins(&dir, &dir.join("key4.db")).unwrap();
    assert_eq!(logins.len(), 2);
    let form = &logins[0];
    assert_eq!(form.input.origin, "https://example.com");
    assert_eq!(form.input.username, "ivan");
    assert_eq!(form.input.password, "пароль-123");
    assert_eq!(
        form.input.form_action_origin.as_deref(),
        Some("https://example.com")
    );
    assert_eq!(form.input.username_field, "user");
    assert_eq!(form.times_used, 5);
    assert_eq!(form.time_created, 1_600_000_000_000);
    assert_eq!(
        form.guid.as_deref(),
        Some("{11111111-2222-4333-8444-555555555555}")
    );
    let realm = &logins[1];
    assert_eq!(realm.input.http_realm.as_deref(), Some("Protected"));
    assert_eq!(realm.input.form_action_origin, None);
    assert_eq!(realm.input.username, "");
    assert_eq!(realm.input.password, "realm-secret");
}

#[test]
fn reports_a_primary_password() {
    let dir = data("nss-primary-password");
    assert!(matches!(
        read_logins(&dir, &dir.join("key4.db")),
        Err(NssError::PrimaryPassword)
    ));
    let key = logins_key(&dir.join("key4.db"), "Сложный пароль 1").unwrap();
    let json = std::fs::read_to_string(dir.join("logins.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let encrypted = value["logins"][0]["encryptedPassword"].as_str().unwrap();
    assert_eq!(key.decrypt(encrypted).unwrap(), "пароль-123");
}
