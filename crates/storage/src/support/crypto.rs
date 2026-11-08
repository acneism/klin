use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};

const KEY_LEN: usize = 32;
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Encryption key is missing.")]
    MissingKey,
    #[error("Encryption key is not valid.")]
    InvalidKey,
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
}

pub type CryptoResult<T> = std::result::Result<T, CryptoError>;

pub trait EncryptorDecryptor: Send + Sync {
    fn encrypt(&self, cleartext: Vec<u8>) -> CryptoResult<Vec<u8>>;
    fn decrypt(&self, ciphertext: Vec<u8>) -> CryptoResult<Vec<u8>>;
}

impl<T: EncryptorDecryptor> EncryptorDecryptor for Arc<T> {
    fn encrypt(&self, cleartext: Vec<u8>) -> CryptoResult<Vec<u8>> {
        (**self).encrypt(cleartext)
    }

    fn decrypt(&self, ciphertext: Vec<u8>) -> CryptoResult<Vec<u8>> {
        (**self).decrypt(ciphertext)
    }
}

pub trait KeyManager: Send + Sync {
    fn get_key(&self) -> CryptoResult<Vec<u8>>;
}

pub struct StaticKeyManager {
    key: String,
}

impl StaticKeyManager {
    pub fn new(key: String) -> Self {
        Self { key }
    }
}

impl KeyManager for StaticKeyManager {
    fn get_key(&self) -> CryptoResult<Vec<u8>> {
        Ok(self.key.as_bytes().into())
    }
}

#[derive(Serialize, Deserialize)]
struct Jwk {
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
    kty: String,
    k: String,
}

#[derive(Serialize, Deserialize)]
struct JweHeader {
    alg: String,
    enc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
}

pub fn create_key() -> String {
    let mut bytes = [0u8; KEY_LEN];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    key_from_bytes(&bytes)
}

pub fn key_from_bytes(bytes: &[u8]) -> String {
    let jwk = Jwk {
        kid: None,
        kty: "oct".into(),
        k: URL_SAFE_NO_PAD.encode(bytes),
    };
    serde_json::to_string(&jwk).unwrap_or_default()
}

struct DirectKey {
    kid: Option<String>,
    secret: Vec<u8>,
}

impl DirectKey {
    fn parse(key: &[u8]) -> CryptoResult<Self> {
        let jwk: Jwk = serde_json::from_slice(key).map_err(|_| CryptoError::InvalidKey)?;
        if jwk.kty != "oct" {
            return Err(CryptoError::InvalidKey);
        }
        let secret = URL_SAFE_NO_PAD
            .decode(&jwk.k)
            .map_err(|_| CryptoError::InvalidKey)?;
        if secret.len() != KEY_LEN {
            return Err(CryptoError::InvalidKey);
        }
        Ok(Self {
            kid: jwk.kid,
            secret,
        })
    }

    fn cipher(&self) -> CryptoResult<Aes256Gcm> {
        Aes256Gcm::new_from_slice(&self.secret).map_err(|_| CryptoError::InvalidKey)
    }

    fn encrypt(&self, data: &[u8]) -> CryptoResult<String> {
        let header = JweHeader {
            alg: "dir".into(),
            enc: "A256GCM".into(),
            kid: self.kid.clone(),
        };
        let header = serde_json::to_string(&header)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
        let header = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let mut iv = [0u8; IV_LEN];
        rand::rngs::OsRng.fill_bytes(&mut iv);
        let mut sealed = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: data,
                    aad: header.as_bytes(),
                },
            )
            .map_err(|_| CryptoError::EncryptionFailed("AES-GCM".into()))?;
        let tag = sealed.split_off(sealed.len() - TAG_LEN);
        Ok(format!(
            "{header}..{}.{}.{}",
            URL_SAFE_NO_PAD.encode(iv),
            URL_SAFE_NO_PAD.encode(&sealed),
            URL_SAFE_NO_PAD.encode(tag)
        ))
    }

    fn decrypt(&self, jwe: &str) -> CryptoResult<Vec<u8>> {
        let fail = |reason: &str| CryptoError::DecryptionFailed(reason.to_owned());
        if jwe.is_empty() {
            return Err(fail("empty ciphertext"));
        }
        let parts: Vec<&str> = jwe.split('.').collect();
        let [header_raw, encrypted_key, iv, ciphertext, tag] = parts[..] else {
            return Err(fail("not a compact JWE"));
        };
        if !encrypted_key.is_empty() {
            return Err(fail("the encrypted key must be empty"));
        }
        let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).map_err(|_| fail("bad base64"));
        let header: JweHeader =
            serde_json::from_slice(&decode(header_raw)?).map_err(|_| fail("bad header"))?;
        if header.alg != "dir" || header.enc != "A256GCM" {
            return Err(fail("unsupported algorithm"));
        }
        let iv = decode(iv)?;
        let tag = decode(tag)?;
        if iv.len() != IV_LEN || tag.len() != TAG_LEN {
            return Err(fail("bad IV or tag length"));
        }
        let mut sealed = decode(ciphertext)?;
        sealed.extend_from_slice(&tag);
        self.cipher()?
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &sealed,
                    aad: header_raw.as_bytes(),
                },
            )
            .map_err(|_| fail("authentication failed"))
    }
}

pub struct ManagedEncryptorDecryptor {
    key_manager: Arc<dyn KeyManager>,
}

impl ManagedEncryptorDecryptor {
    pub fn new(key_manager: Arc<dyn KeyManager>) -> Self {
        Self { key_manager }
    }

    fn key(&self) -> CryptoResult<DirectKey> {
        let key = self.key_manager.get_key().map_err(|e| match e {
            CryptoError::InvalidKey => CryptoError::InvalidKey,
            _ => CryptoError::MissingKey,
        })?;
        DirectKey::parse(&key)
    }
}

impl EncryptorDecryptor for ManagedEncryptorDecryptor {
    fn encrypt(&self, cleartext: Vec<u8>) -> CryptoResult<Vec<u8>> {
        std::str::from_utf8(&cleartext)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
        self.key()?.encrypt(&cleartext).map(String::into_bytes)
    }

    fn decrypt(&self, ciphertext: Vec<u8>) -> CryptoResult<Vec<u8>> {
        let jwe = std::str::from_utf8(&ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;
        let clear = self.key()?.decrypt(jwe)?;
        String::from_utf8(clear)
            .map(String::into_bytes)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }
}

pub fn create_canary(text: &str, key: &str) -> CryptoResult<String> {
    DirectKey::parse(key.as_bytes())?.encrypt(text.as_bytes())
}

pub fn check_canary(canary: &str, text: &str, key: &str) -> CryptoResult<bool> {
    let key = DirectKey::parse(key.as_bytes())?;
    Ok(key
        .decrypt(canary)
        .is_ok_and(|clear| clear == text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_jwe_from_node_jose() {
        let key = key_from_bytes(b"asecret256bitkeyasecret256bitkey");
        let encdec = ManagedEncryptorDecryptor::new(Arc::new(StaticKeyManager::new(key)));
        let jwe = "eyJhbGciOiJkaXIiLCJlbmMiOiJBMjU2R0NNIn0..nhKdQEKqoKPzfCda.rQOj0Nfs6wO5Gj4Quw.CMJFS9YBADLLePdj1sssSg";
        let clear = encdec.decrypt(jwe.as_bytes().to_vec()).unwrap();
        assert_eq!(clear, b"Hello, World!");
    }

    #[test]
    fn round_trips_and_detects_tampering() {
        let encdec = ManagedEncryptorDecryptor::new(Arc::new(StaticKeyManager::new(create_key())));
        let sealed = encdec.encrypt(b"secret password".to_vec()).unwrap();
        assert_eq!(encdec.decrypt(sealed.clone()).unwrap(), b"secret password");
        let mut tampered = String::from_utf8(sealed).unwrap();
        let last = tampered.pop().unwrap();
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        assert!(encdec.decrypt(tampered.into_bytes()).is_err());
    }

    #[test]
    fn rejects_bad_keys_and_checks_canaries() {
        let bad = ManagedEncryptorDecryptor::new(Arc::new(StaticKeyManager::new("bad".into())));
        assert!(matches!(
            bad.encrypt(b"x".to_vec()),
            Err(CryptoError::InvalidKey)
        ));
        let key = create_key();
        let canary = create_canary("canary", &key).unwrap();
        assert!(check_canary(&canary, "canary", &key).unwrap());
        assert!(!check_canary(&canary, "canary", &create_key()).unwrap());
    }
}
