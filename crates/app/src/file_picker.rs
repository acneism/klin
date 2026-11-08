use std::path::PathBuf;
use std::sync::mpsc::Sender;

use klin_shared::{RequestId, TabId};

pub struct Picked {
    pub tab: TabId,
    pub id: RequestId,
    pub files: Option<Vec<PathBuf>>,
}

pub fn open(
    tab: TabId,
    id: RequestId,
    multiple: bool,
    accept: Vec<String>,
    results: Sender<Picked>,
    wake: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let files = pick(multiple, &accept);

        let _ = results.send(Picked { tab, id, files });
        wake();
    });
}

pub fn pick_addon(done: impl FnOnce(Option<PathBuf>) + Send + 'static) {
    std::thread::spawn(move || {
        let file = pick(false, &[".xpi".to_owned(), ".zip".to_owned()])
            .and_then(|files| files.into_iter().next());
        done(file);
    });
}

fn extensions(accept: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for item in accept {
        let item = item.trim().to_ascii_lowercase();
        if let Some(ext) = item.strip_prefix('.') {
            out.push(ext.to_owned());
            continue;
        }
        let known: &[&str] = match item.as_str() {
            "image/*" => &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif", "ico",
            ],
            "image/png" => &["png"],
            "image/jpeg" => &["jpg", "jpeg"],
            "image/gif" => &["gif"],
            "image/webp" => &["webp"],
            "audio/*" => &["mp3", "ogg", "oga", "wav", "flac", "m4a", "opus", "weba"],
            "video/*" => &["mp4", "webm", "ogv", "mkv", "mov"],
            "text/plain" => &["txt"],
            "text/csv" => &["csv"],
            "application/pdf" => &["pdf"],
            "application/json" => &["json"],
            _ => &[],
        };
        out.extend(known.iter().map(|s| s.to_string()));
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(windows)]
fn pick(multiple: bool, accept: &[String]) -> Option<Vec<PathBuf>> {
    let mut dialog = rfd::FileDialog::new();
    let exts = extensions(accept);
    if !exts.is_empty() {
        dialog = dialog
            .add_filter("Подходящие файлы", &exts)
            .add_filter("Все файлы", &["*"]);
    }
    if multiple {
        dialog.pick_files()
    } else {
        dialog.pick_file().map(|f| vec![f])
    }
}

#[cfg(not(windows))]
fn pick(_multiple: bool, accept: &[String]) -> Option<Vec<PathBuf>> {
    log::warn!(
        "file picker is only implemented on Windows (accept: {:?})",
        extensions(accept)
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_to_extensions() {
        let accept = vec![
            ".PNG".to_owned(),
            "image/jpeg".to_owned(),
            "application/x-unknown".to_owned(),
        ];
        assert_eq!(extensions(&accept), ["jpeg", "jpg", "png"]);
    }
}
