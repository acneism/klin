use klin_shared::Url;

const DOWNLOAD_EXTENSIONS: &[&str] = &[
    "zip",
    "rar",
    "7z",
    "tar",
    "gz",
    "tgz",
    "bz2",
    "tbz2",
    "xz",
    "txz",
    "zst",
    "cab",
    "lz",
    "lzma",
    "exe",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "apk",
    "jar",
    "appimage",
    "flatpakref",
    "snap",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "vmdk",
    "doc",
    "docx",
    "xls",
    "xlsx",
    "ppt",
    "pptx",
    "odt",
    "ods",
    "odp",
    "rtf",
    "epub",
    "pdf",
    "torrent",
    "bin",
    "dll",
    "sys",
    "ttf",
    "otf",
    "woff2",
];

pub fn looks_like_download(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(name) = url.path_segments().and_then(|mut s| s.next_back()) else {
        return false;
    };
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    DOWNLOAD_EXTENSIONS.contains(&ext.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(s: &str) -> bool {
        looks_like_download(&Url::parse(s).unwrap())
    }

    #[test]
    fn detects_by_extension() {
        assert!(check("https://example.com/files/setup.EXE"));
        assert!(check("https://example.com/a/b/archive.tar.gz?token=1"));
        assert!(check("http://example.com/doc.pdf#page=2"));
        assert!(!check("https://example.com/"));
        assert!(!check("https://example.com/page.html"));
        assert!(!check("https://example.com/video.mp4"));
        assert!(!check("https://example.com/zip"));
        assert!(!check("file:///C:/x.zip"));
    }
}
