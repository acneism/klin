use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::host::BACKGROUND_PAGE;
use crate::package::Package;

type FileMap = HashMap<String, (Package, Option<String>)>;

#[derive(Clone, Default)]
pub struct ExtensionFiles(Arc<RwLock<FileMap>>);

pub fn mime(path: &str) -> &'static str {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" | "xhtml" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

impl ExtensionFiles {
    pub fn insert(&self, uuid: &str, package: Package, background: Option<String>) {
        if let Ok(mut map) = self.0.write() {
            map.insert(uuid.to_owned(), (package, background));
        }
    }

    pub fn remove(&self, uuid: &str) {
        if let Ok(mut map) = self.0.write() {
            map.remove(uuid);
        }
    }

    pub fn get(&self, uuid: &str, path: &str) -> Option<(&'static str, Vec<u8>)> {
        let map = self.0.read().ok()?;
        let (package, background) = map.get(uuid)?;
        let path = path.trim_start_matches('/');
        if path == BACKGROUND_PAGE
            && let Some(html) = background
        {
            return Some((mime(path), html.clone().into_bytes()));
        }
        let path = if path.is_empty() { "index.html" } else { path };
        package.read(path).ok().map(|bytes| (mime(path), bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_files_and_the_generated_background_page() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("popup.html"), "<p>hi</p>").unwrap();
        let files = ExtensionFiles::default();
        files.insert("u", Package::open(dir.path()).unwrap(), Some("<bg>".into()));
        assert_eq!(
            files.get("u", "/popup.html"),
            Some(("text/html; charset=utf-8", b"<p>hi</p>".to_vec()))
        );
        assert_eq!(
            files
                .get("u", "/_generated_background_page.html")
                .unwrap()
                .1,
            b"<bg>"
        );
        assert!(files.get("u", "/../secret").is_none());
        assert!(files.get("other", "/popup.html").is_none());
        files.remove("u");
        assert!(files.get("u", "/popup.html").is_none());
    }
}
