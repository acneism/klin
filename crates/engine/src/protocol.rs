use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use klin_internal_pages::reader::{self, Article};
use klin_shared::Url;
use servo::protocol_handler::{
    DoneChannel, FetchContext, HttpStatus, NetworkError, ProtocolHandler, Request,
    ResourceFetchTiming, Response, ResponseBody,
};

pub fn internal_url(url: &Url) -> Url {
    klin_internal_pages::about_to_internal(url.as_str())
        .and_then(|s| Url::parse(&s).ok())
        .unwrap_or_else(|| url.clone())
}

pub(crate) fn display_url(url: Url) -> Url {
    klin_internal_pages::internal_to_about(url.as_str())
        .and_then(|s| Url::parse(&s).ok())
        .unwrap_or(url)
}

const READER_CACHE: usize = 16;

#[derive(Clone, Default)]
pub(crate) struct ReaderStore(Arc<Mutex<VecDeque<Article>>>);

impl ReaderStore {
    pub fn insert(&self, article: Article) {
        let Ok(mut articles) = self.0.lock() else {
            return;
        };
        articles.retain(|a| a.url != article.url);
        articles.push_front(article);
        articles.truncate(READER_CACHE);
    }

    fn get(&self, url: &str) -> Option<Article> {
        let articles = self.0.lock().ok()?;
        articles.iter().find(|a| a.url == url).cloned()
    }
}

pub(crate) struct InternalPages {
    pub reader: ReaderStore,
}

fn html_response(request: &Request, url: servo::ServoUrl, body: Vec<u8>, mime: &str) -> Response {
    let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
    *response.body.lock() = ResponseBody::Done(body);
    if let Ok(value) = http::HeaderValue::from_str(mime) {
        response.headers.insert(http::header::CONTENT_TYPE, value);
    }
    response.status = HttpStatus::default();
    response
}

impl ProtocolHandler for InternalPages {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let response = if url.path() == klin_internal_pages::READER {
            let original = url
                .as_url()
                .query_pairs()
                .find(|(key, _)| key == "url")
                .map(|(_, value)| value.into_owned())
                .unwrap_or_default();
            let article = self.reader.get(&original);
            let body = reader::render(article.as_ref(), &original);
            let mut response =
                html_response(request, url, body.into_bytes(), "text/html; charset=utf-8");
            if let Ok(value) = http::HeaderValue::from_str(reader::CSP) {
                response
                    .headers
                    .insert(http::header::CONTENT_SECURITY_POLICY, value);
            }
            response
        } else {
            match klin_internal_pages::page(url.path()) {
                Some((mime, body)) => html_response(request, url, body.to_vec(), mime),
                None => {
                    Response::network_error(NetworkError::ResourceLoadError("unknown page".into()))
                }
            }
        };
        Box::pin(std::future::ready(response))
    }
}

pub(crate) struct Resources;

impl ProtocolHandler for Resources {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let host = url.host_str().unwrap_or_default().to_owned();
        let response = match klin_internal_pages::resources::resource(&host, url.path()) {
            Some((mime, body)) => {
                let mut response = html_response(request, url, body.to_vec(), mime);
                response.headers.insert(
                    http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                    http::HeaderValue::from_static("*"),
                );
                response
            }
            None => {
                Response::network_error(NetworkError::ResourceLoadError("unknown resource".into()))
            }
        };
        Box::pin(std::future::ready(response))
    }

    fn is_fetchable(&self) -> bool {
        true
    }

    fn is_secure(&self) -> bool {
        true
    }
}
