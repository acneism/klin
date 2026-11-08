use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("не удалось прочитать {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("архив дополнения повреждён: {0}")]
    Zip(String),
    #[error("в дополнении нет файла {0}")]
    Missing(String),
}

#[derive(Debug, Clone)]
pub enum Package {
    Directory(PathBuf),
    Archive(std::sync::Arc<HashMap<String, Vec<u8>>>),
}

const MAX_ARCHIVE_SIZE: u64 = 256 * 1024 * 1024;

pub fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl Package {
    pub fn open(path: &Path) -> Result<Self, PackageError> {
        if path.is_dir() {
            return Ok(Package::Directory(path.to_owned()));
        }
        let file = std::fs::File::open(path).map_err(|e| PackageError::Io(path.to_owned(), e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| PackageError::Zip(e.to_string()))?;
        let mut files = HashMap::new();
        let mut total = 0u64;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| PackageError::Zip(e.to_string()))?;
            if entry.is_dir() {
                continue;
            }
            let Some(name) = entry.name().ok().and_then(|name| normalize(&name)) else {
                continue;
            };
            total += entry.size();
            if total > MAX_ARCHIVE_SIZE {
                return Err(PackageError::Zip("слишком большой архив".into()));
            }
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut data)
                .map_err(|e| PackageError::Zip(e.to_string()))?;
            files.insert(name, data);
        }
        Ok(Package::Archive(std::sync::Arc::new(files)))
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, PackageError> {
        let name = normalize(path).ok_or_else(|| PackageError::Missing(path.to_owned()))?;
        match self {
            Package::Directory(dir) => {
                let full = dir.join(&name);
                std::fs::read(&full).map_err(|_| PackageError::Missing(name))
            }
            Package::Archive(files) => files.get(&name).cloned().ok_or(PackageError::Missing(name)),
        }
    }

    pub fn read_text(&self, path: &str) -> Result<String, PackageError> {
        let bytes = self.read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text.trim_start_matches('\u{feff}').to_owned())
    }

    pub fn exists(&self, path: &str) -> bool {
        self.read(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize("/a/./b//c.js").as_deref(), Some("a/b/c.js"));
        assert_eq!(normalize("a/../b.js").as_deref(), Some("b.js"));
        assert_eq!(normalize("../etc/passwd"), None);
        assert_eq!(normalize("/"), None);
    }

    #[test]
    fn reads_xpi_archives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.xpi");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(b"\xef\xbb\xbf{}").unwrap();
        zip.start_file("js/a.js", options).unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let package = Package::open(&path).unwrap();
        assert_eq!(package.read_text("manifest.json").unwrap(), "{}");
        assert_eq!(package.read("/js/a.js").unwrap(), b"x");
        assert!(package.read("missing").is_err());
    }
}
