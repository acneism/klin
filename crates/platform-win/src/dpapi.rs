use std::io;

const ENTROPY: &[u8] = b"Klin logins key";

pub const PROTECTS: bool = cfg!(windows);

#[cfg(windows)]
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    windows_dpapi::encrypt_data(data, windows_dpapi::Scope::User, Some(ENTROPY))
        .map_err(|e| io::Error::other(format!("{e:#}")))
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    windows_dpapi::decrypt_data(data, windows_dpapi::Scope::User, Some(ENTROPY))
        .map_err(|e| io::Error::other(format!("{e:#}")))
}

#[cfg(not(windows))]
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    let _ = ENTROPY;
    Ok(data.to_vec())
}

#[cfg(not(windows))]
pub fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let sealed = protect(b"secret").unwrap();
        assert_eq!(unprotect(&sealed).unwrap(), b"secret");
        if PROTECTS {
            assert_ne!(sealed, b"secret");
        }
    }
}
