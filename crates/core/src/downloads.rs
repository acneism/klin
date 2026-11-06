//! Какие переходы считать загрузкой файла.
//!
//! Отступление от Firefox: Firefox решает по ответу сервера (`Content-Disposition: attachment`
//! и тип содержимого, который не умеет показать). Перехват запросов в Servo срабатывает
//! до ответа, а патчи в Servo мы не отправляем (правила Servo запрещают код, написанный ИИ),
//! поэтому загрузку распознаём по расширению в адресе.

use klin_shared::Url;

/// Расширения файлов, которые браузер не показывает, а сохраняет.
/// PDF — тоже загрузка, пока нет встроенного просмотра (PDF.js, версия 0.3).
const DOWNLOAD_EXTENSIONS: &[&str] = &[
    // архивы
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
    // программы и пакеты
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
    // образы дисков
    "iso",
    "img",
    "vhd",
    "vhdx",
    "vmdk",
    // документы, которые браузер не показывает
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
    // прочее
    "torrent",
    "bin",
    "dll",
    "sys",
    "ttf",
    "otf",
    "woff2",
];

/// Похож ли адрес на файл для загрузки.
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
