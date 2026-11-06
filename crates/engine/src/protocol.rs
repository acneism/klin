use std::future::Future;
use std::pin::Pin;

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

pub(crate) struct InternalPages;

impl ProtocolHandler for InternalPages {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        let response = match klin_internal_pages::page(url.path()) {
            Some((mime, body)) => {
                let mut response =
                    Response::new(url, ResourceFetchTiming::new(request.timing_type()));
                *response.body.lock() = ResponseBody::Done(body.to_vec());
                if let Ok(value) = http::HeaderValue::from_str(mime) {
                    response.headers.insert(http::header::CONTENT_TYPE, value);
                }
                response.status = HttpStatus::default();
                response
            }
            None => Response::network_error(NetworkError::ResourceLoadError("unknown page".into())),
        };
        Box::pin(std::future::ready(response))
    }
}
