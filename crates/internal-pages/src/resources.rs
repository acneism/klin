use include_dir::{Dir, include_dir};

static RESOURCES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/resources");

pub const SCHEME: &str = "klin-res";

pub const PDF_VIEWER_HTML: &[u8] = include_bytes!("../resources/pdfviewer/viewer.html");

fn mime(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext {
        "mjs" | "js" => "text/javascript",
        "css" => "text/css",
        "html" => "text/html",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "png" => "image/png",
        "wasm" => "application/wasm",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

pub fn resource(host: &str, path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = path.trim_start_matches('/');
    if host.is_empty()
        || path.is_empty()
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return None;
    }
    let file = RESOURCES.get_file(format!("{host}/{path}"))?;
    Some((mime(path), file.contents()))
}

pub fn is_pdf_url(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_pdfjs_files_with_types() {
        let (mime, body) = resource("pdfjs", "/build/pdf.min.mjs").unwrap();
        assert_eq!(mime, "text/javascript");
        assert!(!body.is_empty());
        assert_eq!(
            resource("pdfjs", "/wasm/openjpeg.wasm").unwrap().0,
            "application/wasm"
        );
        assert!(resource("pdfjs", "/cmaps/78-EUC-H.bcmap").is_some());
        assert!(resource("pdfviewer", "/viewer.mjs").is_some());
    }

    #[test]
    fn rejects_traversal_and_unknown_files() {
        assert!(resource("pdfjs", "/../pdfviewer/viewer.mjs").is_none());
        assert!(resource("pdfjs", "/build//pdf.min.mjs").is_none());
        assert!(resource("", "/x").is_none());
        assert!(resource("pdfjs", "/missing.js").is_none());
    }

    #[test]
    fn recognizes_pdf_paths() {
        assert!(is_pdf_url("/docs/Report.PDF"));
        assert!(!is_pdf_url("/docs/pdf"));
        assert!(!is_pdf_url("/a.pdf/view"));
    }
}
