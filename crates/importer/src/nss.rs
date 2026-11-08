use std::path::Path;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OpenFlags};
use sha1::{Digest, Sha1};

const PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
const PBKDF2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0c];
const HMAC_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x09];
const AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a];
const DES_EDE3_CBC: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x03, 0x07];
const PBE_SHA1_3DES: &[u8] = &[
    0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x0c, 0x05, 0x01, 0x03,
];
const LOGINS_KEY_ID: [u8; 16] = [0xf8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
const PASSWORD_CHECK: &[u8] = b"password-check";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NssError {
    #[error("файл key4.db повреждён или имеет неизвестный формат")]
    Format,
    #[error("профиль Firefox защищён основным паролем")]
    PrimaryPassword,
    #[error("в key4.db нет ключа для паролей")]
    NoKey,
    #[error("не удалось расшифровать данные")]
    Decrypt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Der<'a> {
    Sequence(Vec<Der<'a>>),
    Octets(&'a [u8]),
    Oid(&'a [u8]),
    Integer(&'a [u8]),
    Other,
}

fn parse_der(input: &[u8]) -> Option<(Der<'_>, &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first & 0x80 == 0 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let len = rest[..count]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        (len, &rest[count..])
    };
    if rest.len() < len {
        return None;
    }
    let (body, rest) = rest.split_at(len);
    let value = match tag {
        0x30 => {
            let mut items = Vec::new();
            let mut body = body;
            while !body.is_empty() {
                let (item, next) = parse_der(body)?;
                items.push(item);
                body = next;
            }
            Der::Sequence(items)
        }
        0x04 => Der::Octets(body),
        0x06 => Der::Oid(body),
        0x02 => Der::Integer(body),
        _ => Der::Other,
    };
    Some((value, rest))
}

fn der(input: &[u8]) -> Result<Der<'_>, NssError> {
    parse_der(input).map(|(d, _)| d).ok_or(NssError::Format)
}

fn integer(bytes: &[u8]) -> Option<u32> {
    if bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b)))
}

fn aes_cbc(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, NssError> {
    cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(|_| NssError::Decrypt)?
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| NssError::Decrypt)
}

fn des3_cbc(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, NssError> {
    let key = key.get(..24).ok_or(NssError::Decrypt)?;
    cbc::Decryptor::<des::TdesEde3>::new_from_slices(key, iv)
        .map_err(|_| NssError::Decrypt)?
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| NssError::Decrypt)
}

fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

fn decrypt_pbe(item: &[u8], global_salt: &[u8], password: &[u8]) -> Result<Vec<u8>, NssError> {
    let Der::Sequence(top) = der(item)? else {
        return Err(NssError::Format);
    };
    let [Der::Sequence(algorithm), Der::Octets(ciphertext)] = &top[..] else {
        return Err(NssError::Format);
    };
    let hashed_password = Sha1::new()
        .chain_update(global_salt)
        .chain_update(password)
        .finalize();
    match &algorithm[..] {
        [Der::Oid(oid), Der::Sequence(params)] if *oid == PBES2 => {
            let [Der::Sequence(kdf), Der::Sequence(cipher)] = &params[..] else {
                return Err(NssError::Format);
            };
            let [Der::Oid(kdf_oid), Der::Sequence(kdf_params)] = &kdf[..] else {
                return Err(NssError::Format);
            };
            if *kdf_oid != PBKDF2 {
                return Err(NssError::Format);
            }
            let (salt, iterations, key_len, prf) = match &kdf_params[..] {
                [
                    Der::Octets(salt),
                    Der::Integer(iterations),
                    Der::Integer(key_len),
                    Der::Sequence(prf),
                ] => (*salt, *iterations, Some(*key_len), prf.as_slice()),
                [
                    Der::Octets(salt),
                    Der::Integer(iterations),
                    Der::Sequence(prf),
                ] => (*salt, *iterations, None, prf.as_slice()),
                _ => return Err(NssError::Format),
            };
            if !matches!(prf.first(), Some(Der::Oid(oid)) if *oid == HMAC_SHA256) {
                return Err(NssError::Format);
            }
            let iterations = integer(iterations).ok_or(NssError::Format)?;
            let key_len = key_len.and_then(integer).unwrap_or(32) as usize;
            if key_len != 32 || iterations == 0 {
                return Err(NssError::Format);
            }
            let [Der::Oid(cipher_oid), Der::Octets(iv)] = &cipher[..] else {
                return Err(NssError::Format);
            };
            if *cipher_oid != AES256_CBC {
                return Err(NssError::Format);
            }
            let mut key = [0u8; 32];
            pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&hashed_password, salt, iterations, &mut key);
            let iv: Vec<u8> = if iv.len() == 14 {
                [&[0x04, 0x0e][..], iv].concat()
            } else {
                iv.to_vec()
            };
            aes_cbc(&key, &iv, ciphertext)
        }
        [Der::Oid(oid), Der::Sequence(params)] if *oid == PBE_SHA1_3DES => {
            let [Der::Octets(entry_salt), Der::Integer(_)] = &params[..] else {
                return Err(NssError::Format);
            };
            let mut padded_salt = entry_salt.to_vec();
            padded_salt.resize(20.max(entry_salt.len()), 0);
            let chp = Sha1::new()
                .chain_update(hashed_password)
                .chain_update(entry_salt)
                .finalize();
            let k1 = hmac_sha1(&chp, &[&padded_salt, entry_salt]);
            let tk = hmac_sha1(&chp, &[&padded_salt]);
            let k2 = hmac_sha1(&chp, &[&tk, entry_salt]);
            let k = [k1, k2].concat();
            des3_cbc(&k[..24], &k[k.len() - 8..], ciphertext)
        }
        _ => Err(NssError::Format),
    }
}

pub struct LoginsKey(Vec<u8>);

pub fn logins_key(key4: &Path, primary_password: &str) -> Result<LoginsKey, NssError> {
    let conn = Connection::open_with_flags(key4, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| NssError::Format)?;
    let (global_salt, check): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT item1, item2 FROM metaData WHERE id = 'password'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| NssError::Format)?;
    let password = primary_password.as_bytes();
    match decrypt_pbe(&check, &global_salt, password) {
        Ok(clear) if clear.starts_with(PASSWORD_CHECK) => {}
        Ok(_) | Err(NssError::Decrypt) => return Err(NssError::PrimaryPassword),
        Err(error) => return Err(error),
    }
    let mut stmt = conn
        .prepare("SELECT a11, a102 FROM nssPrivate")
        .map_err(|_| NssError::Format)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|_| NssError::Format)?;
    for (value, id) in rows.flatten() {
        if id == LOGINS_KEY_ID {
            return decrypt_pbe(&value, &global_salt, password).map(LoginsKey);
        }
    }
    Err(NssError::NoKey)
}

impl LoginsKey {
    pub fn decrypt(&self, encrypted: &str) -> Result<String, NssError> {
        let raw = STANDARD
            .decode(encrypted.trim())
            .map_err(|_| NssError::Format)?;
        let Der::Sequence(top) = der(&raw)? else {
            return Err(NssError::Format);
        };
        let [
            Der::Octets(key_id),
            Der::Sequence(algorithm),
            Der::Octets(ciphertext),
        ] = &top[..]
        else {
            return Err(NssError::Format);
        };
        if *key_id != LOGINS_KEY_ID {
            return Err(NssError::NoKey);
        }
        let [Der::Oid(oid), Der::Octets(iv)] = &algorithm[..] else {
            return Err(NssError::Format);
        };
        let clear = if *oid == DES_EDE3_CBC {
            des3_cbc(&self.0, iv, ciphertext)?
        } else if *oid == AES256_CBC {
            aes_cbc(self.0.get(..32).ok_or(NssError::Decrypt)?, iv, ciphertext)?
        } else {
            return Err(NssError::Format);
        };
        String::from_utf8(clear).map_err(|_| NssError::Decrypt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_der_lengths() {
        let long = [0x04, 0x81, 0x03, 1, 2, 3];
        assert_eq!(der(&long).unwrap(), Der::Octets(&[1, 2, 3]));
        assert!(der(&[0x04, 0x05, 1]).is_err());
        assert!(der(&[0x30, 0x02, 0x04]).is_err());
    }

    #[test]
    fn decrypts_legacy_pbe_sha1_3des() {
        use cbc::cipher::BlockEncryptMut;
        let global_salt = b"global-salt-0123456789";
        let entry_salt = [7u8; 20];
        let hp = Sha1::new().chain_update(global_salt).finalize();
        let chp = Sha1::new()
            .chain_update(hp)
            .chain_update(entry_salt)
            .finalize();
        let k1 = hmac_sha1(&chp, &[&entry_salt, &entry_salt]);
        let tk = hmac_sha1(&chp, &[&entry_salt]);
        let k2 = hmac_sha1(&chp, &[&tk, &entry_salt]);
        let k = [k1, k2].concat();
        let ciphertext = cbc::Encryptor::<des::TdesEde3>::new_from_slices(&k[..24], &k[32..])
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(b"password-check");
        let tlv = |tag: u8, content: &[u8]| {
            let mut out = vec![tag, content.len() as u8];
            out.extend_from_slice(content);
            out
        };
        let params = [tlv(0x04, &entry_salt), tlv(0x02, &[1])].concat();
        let algorithm = [tlv(0x06, PBE_SHA1_3DES), tlv(0x30, &params)].concat();
        let item = tlv(
            0x30,
            &[tlv(0x30, &algorithm), tlv(0x04, &ciphertext)].concat(),
        );
        let clear = decrypt_pbe(&item, global_salt, b"").unwrap();
        assert_eq!(clear, b"password-check");
        assert!(decrypt_pbe(&item, global_salt, b"wrong").is_err());
    }
}
