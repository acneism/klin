//! Служба загрузок — по образцу менеджера загрузок Firefox (`DownloadCore`, `DownloadPaths`).
//!
//! Каждая загрузка идёт в своём потоке: данные пишутся в `<имя>.part`, по завершении файл
//! переименовывается. Как в Firefox по умолчанию (`browser.download.useDownloadDir`),
//! файл сразу сохраняется в папку загрузок без вопроса, куда сохранить.

mod names;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use klin_shared::download::DownloadEvent;
use klin_shared::{DownloadId, Url};
use ureq::ResponseExt;

pub use names::{file_name_from_content_disposition, file_name_from_url, sanitize, unique_path};

/// Что нужно, чтобы скачать файл так же, как его запросил бы движок.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    pub url: Url,
    pub referrer: Option<Url>,
    /// Заголовок `Cookie` из хранилища движка.
    pub cookie: Option<String>,
    pub user_agent: String,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct DownloadService {
    dir: PathBuf,
    events: Sender<DownloadEvent>,
    wake: Wake,
    cancel: Arc<Mutex<HashMap<DownloadId, Arc<AtomicBool>>>>,
}

impl DownloadService {
    /// `events` получает сообщения о ходе загрузок; после каждого вызывается `wake`.
    pub fn new(
        dir: PathBuf,
        events: Sender<DownloadEvent>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            dir,
            events,
            wake: Arc::new(wake),
            cancel: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Начать загрузку. Имя файла станет известно из ответа сервера.
    pub fn start(&self, request: DownloadRequest) -> DownloadId {
        let id = DownloadId::next();
        let flag = Arc::new(AtomicBool::new(false));
        self.cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, flag.clone());
        let job = Job {
            id,
            request,
            dir: self.dir.clone(),
            events: self.events.clone(),
            wake: self.wake.clone(),
            canceled: flag,
            registry: self.cancel.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name(format!("download-{}", id.get()))
            .spawn(move || job.run());
        if let Err(e) = spawned {
            log::error!("download: failed to start thread: {e}");
        }
        id
    }

    /// Отменить загрузку; недокачанный `.part` удаляется.
    pub fn cancel(&self, id: DownloadId) {
        if let Some(flag) = self
            .cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
        {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

struct Job {
    id: DownloadId,
    request: DownloadRequest,
    dir: PathBuf,
    events: Sender<DownloadEvent>,
    wake: Wake,
    canceled: Arc<AtomicBool>,
    registry: Arc<Mutex<HashMap<DownloadId, Arc<AtomicBool>>>>,
}

enum Outcome {
    Finished(u64),
    Canceled,
}

impl Job {
    fn send(&self, event: DownloadEvent) {
        if self.events.send(event).is_ok() {
            (self.wake)();
        }
    }

    fn run(self) {
        let id = self.id;
        let mut part = None;
        let result = self.download(&mut part);
        match result {
            Ok(Outcome::Finished(size)) => self.send(DownloadEvent::Finished { id, size }),
            Ok(Outcome::Canceled) => {
                if let Some(p) = &part {
                    let _ = fs::remove_file(p);
                }
                self.send(DownloadEvent::Canceled { id });
            }
            Err(error) => {
                log::warn!("download {id} failed: {error}");
                if let Some(p) = &part {
                    let _ = fs::remove_file(p);
                }
                self.send(DownloadEvent::Failed { id, error });
            }
        }
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }

    fn download(&self, part_out: &mut Option<PathBuf>) -> Result<Outcome, String> {
        let request = &self.request;
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .http_status_as_error(true)
            .build()
            .new_agent();
        let mut call = agent
            .get(request.url.as_str())
            .header("User-Agent", &request.user_agent);
        if let Some(cookie) = &request.cookie {
            call = call.header("Cookie", cookie);
        }
        if let Some(referrer) = &request.referrer {
            call = call.header("Referer", referrer.as_str());
        }
        let response = call.call().map_err(|e| e.to_string())?;

        let total = response.body().content_length();
        let from_header = response
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .and_then(file_name_from_content_disposition);
        let final_url =
            Url::parse(&response.get_uri().to_string()).unwrap_or_else(|_| request.url.clone());
        let name = sanitize(&from_header.unwrap_or_else(|| file_name_from_url(&final_url)));

        fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let path = unique_path(&self.dir, &name);
        let part = part_path(&path);
        // Занять имя сразу, чтобы параллельная загрузка с тем же именем выбрала другое.
        File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        *part_out = Some(part.clone());
        self.send(DownloadEvent::Started {
            id: self.id,
            url: request.url.clone(),
            path: path.clone(),
            total,
        });

        let mut file = File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut reader = response.into_body().into_reader();
        let mut buffer = vec![0; 64 * 1024];
        let mut received = 0u64;
        let mut last_report = Instant::now();
        loop {
            if self.canceled.load(Ordering::Relaxed) {
                drop(file);
                let _ = fs::remove_file(&path);
                return Ok(Outcome::Canceled);
            }
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            received += n as u64;
            // Не чаще 10 раз в секунду: интерфейсу этого хватает.
            if last_report.elapsed() >= Duration::from_millis(100) {
                last_report = Instant::now();
                self.send(DownloadEvent::Progress {
                    id: self.id,
                    received,
                    total,
                });
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        *part_out = None;
        Ok(Outcome::Finished(received))
    }
}

/// `файл.zip` → `файл.zip.part`, как у Firefox.
fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Папка «Загрузки» пользователя.
/// TODO(platform-win): SHGetKnownFolderPath(FOLDERID_Downloads) — папка может быть перенесена.
pub fn default_download_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Downloads")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::mpsc::channel;

    /// HTTP-сервер на один запрос; возвращает адрес и полученные заголовки запроса.
    fn serve_once(response: Vec<u8>) -> (String, std::sync::mpsc::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut lines = Vec::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                lines.push(line.trim().to_owned());
            }
            stream.write_all(&response).unwrap();
            tx.send(lines).unwrap();
        });
        (addr, rx)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("klin-downloads-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn downloads_with_cookie_and_content_disposition() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let body = b"hello, klin";
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename*=UTF-8''%D0%BE%D1%82%D1%87%D1%91%D1%82.txt\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        let (addr, headers) = serve_once(response);

        let dir = temp_dir("cd");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("отчёт.txt"), "уже есть").unwrap();
        let (tx, rx) = channel();
        let service = DownloadService::new(dir.clone(), tx, || {});
        let id = service.start(DownloadRequest {
            url: Url::parse(&format!("{addr}/get?id=1")).unwrap(),
            referrer: None,
            cookie: Some("session=abc".into()),
            user_agent: "Klin-test".into(),
        });

        let events: Vec<DownloadEvent> = rx
            .iter()
            .take_while(|e| {
                !matches!(
                    e,
                    DownloadEvent::Finished { .. } | DownloadEvent::Failed { .. }
                )
            })
            .collect();
        let DownloadEvent::Started { path, total, .. } = &events[0] else {
            panic!("{events:?}")
        };
        assert_eq!(path, &dir.join("отчёт(1).txt"));
        assert_eq!(*total, Some(body.len() as u64));
        assert_eq!(fs::read(path).unwrap(), body);
        assert!(!part_path(path).exists());
        let headers = headers.recv().unwrap();
        assert!(
            headers
                .iter()
                .any(|h| h.eq_ignore_ascii_case("cookie: session=abc")),
            "{headers:?}"
        );
        assert!(
            headers
                .iter()
                .any(|h| h.eq_ignore_ascii_case("user-agent: Klin-test"))
        );
        let _ = id;
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_error_is_reported_and_nothing_is_left() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (addr, _) = serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        let dir = temp_dir("404");
        let (tx, rx) = channel();
        let service = DownloadService::new(dir.clone(), tx, || {});
        service.start(DownloadRequest {
            url: Url::parse(&format!("{addr}/missing.zip")).unwrap(),
            referrer: None,
            cookie: None,
            user_agent: "Klin-test".into(),
        });
        assert!(matches!(rx.recv().unwrap(), DownloadEvent::Failed { .. }));
        assert!(fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0);
        let _ = fs::remove_dir_all(dir);
    }
}
