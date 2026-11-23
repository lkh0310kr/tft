//! The HTTP client for remote images: a plain `GET` with no cookies or
//! referrer, on a worker thread, never the UI thread.

use futures::future::BoxFuture;
use http_client::http::{HeaderValue, Request, Response};
use http_client::{AsyncBody, HttpClient, Url};
use std::time::Duration;

/// What an image request accepts: Chromium's own list for `<img>`, which is
/// what Obsidian's requests carry.
const ACCEPT: &str = "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8";

/// The largest body read. A remote image bigger than this is a failed load,
/// drawn as its alt text, rather than an unbounded allocation.
const MAX_BODY: u64 = 64 * 1024 * 1024;

pub struct RemoteClient {
    agent: ureq::Agent,
    user_agent: HeaderValue,
}

impl RemoteClient {
    pub fn new() -> RemoteClient {
        let user_agent = format!("TFT/{}", env!("CARGO_PKG_VERSION"));
        let agent = ureq::Agent::config_builder()
            // A 404 is an answer with a status, not a transport error: the
            // loader reads the status and fails the image itself.
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(user_agent.as_str())
            .build()
            .into();
        RemoteClient {
            agent,
            user_agent: HeaderValue::from_str(&user_agent).expect("an ASCII user agent"),
        }
    }
}

impl HttpClient for RemoteClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let agent = self.agent.clone();
        let uri = req.uri().to_string();
        let method = req.method().clone();
        let (tx, rx) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("tft-remote-fetch".into())
            .spawn(move || {
                let _ = tx.send(fetch(&agent, &method, &uri));
            });
        Box::pin(async move {
            spawned?;
            rx.await?
        })
    }
}

/// One `GET` of an `http:` or `https:` URL; anything else is refused, so a
/// `file:` or `data:` destination can never reach the network layer here.
fn fetch(
    agent: &ureq::Agent,
    method: &http_client::http::Method,
    uri: &str,
) -> anyhow::Result<Response<AsyncBody>> {
    if method != http_client::http::Method::GET {
        anyhow::bail!("only GET is supported, not {method}");
    }
    if !is_remote(uri) {
        anyhow::bail!("not an http(s) URL: {uri}");
    }
    let response = agent.get(uri).header("Accept", ACCEPT).call()?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()?;
    Ok(Response::builder()
        .status(status)
        .body(AsyncBody::from(body))?)
}

/// What a download brought back: the response's `Content-Type`, if any, and
/// the body.
pub struct Download {
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// One blocking `GET` for `editor:download-attachments`: the same agent
/// settings as an image load, and a status other than 2xx is a failure, as
/// a 404 was for Obsidian 1.15.0 (the run stopped there). Run it off the UI
/// thread.
pub fn download(url: &str) -> anyhow::Result<Download> {
    if !is_remote(url) {
        anyhow::bail!("not an http(s) URL: {url}");
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(format!("TFT/{}", env!("CARGO_PKG_VERSION")).as_str())
        .build()
        .into();
    let response = agent.get(url).header("Accept", ACCEPT).call()?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        anyhow::bail!("{url} answered {status}");
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .into_body()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()?;
    Ok(Download {
        content_type,
        bytes,
    })
}

pub use tft_core::remote::is_remote;
