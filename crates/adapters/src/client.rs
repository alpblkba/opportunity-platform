use std::sync::Arc;
use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::StatusCode;
use url::Url;

use crate::net;

/// Identifies this tool to the boards it reads. A real contact string matters more than a browser impersonation string: the goal is to be a well-behaved documented-API client, not to look like a person.
const USER_AGENT: &str = concat!(
    "opportunity-platform/",
    env!("CARGO_PKG_VERSION"),
    " (personal job discovery; read-only public job board API client)"
);

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("blocked outbound request: {0}")]
    Blocked(#[from] net::UrlError),
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("response body was not valid json: {0}")]
    Json(String),
}

#[derive(Debug)]
pub struct Response {
    pub status: StatusCode,
    pub body: String,
    pub final_url: String,
}

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    max_body_bytes: usize,
}

impl HttpClient {
    pub fn new() -> Result<Self, HttpError> {
        // Every redirect hop is re-checked, so a source cannot bounce the fetcher onto a private address after the initial allowlist check.
        let policy = Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 {
                return attempt.stop();
            }
            match net::check_public_https(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(_) => attempt.stop(),
            }
        });
        let inner = reqwest::Client::builder()
            // Resolution itself refuses private addresses, which covers the initial request and every redirect hop with no window between checking a name and dialling it.
            .dns_resolver(Arc::new(net::GuardedResolver))
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .redirect(policy)
            .build()
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        Ok(Self {
            inner,
            max_body_bytes: 8 * 1024 * 1024,
        })
    }

    /// Read-only GET with the destination pinned to the provider's documented API host.
    pub async fn get_json(&self, url: &str, allowed_hosts: &[&str]) -> Result<Response, HttpError> {
        let parsed = Url::parse(url)?;
        net::check_allowed_host(&parsed, allowed_hosts)?;
        self.send(self.inner.get(parsed).header("accept", "application/json"))
            .await
    }

    /// Read-only GET of an owner-registered page on a host this tool does not control. There is no host allowlist here by design, which is exactly why the resolver refuses private destinations.
    pub async fn get_text(&self, url: &str) -> Result<Response, HttpError> {
        let parsed = Url::parse(url)?;
        net::check_public_https(&parsed)?;
        self.send(self.inner.get(parsed).header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        ))
        .await
    }

    /// Read-only JSON query against a search endpoint that only accepts POST. The body is a query, never a submission: nothing is created or changed on the far side.
    pub async fn post_json(
        &self,
        url: &str,
        body: serde_json::Value,
        allowed_host_suffix: &str,
    ) -> Result<Response, HttpError> {
        let parsed = Url::parse(url)?;
        net::check_host_suffix(&parsed, allowed_host_suffix)?;
        self.send(
            self.inner
                .post(parsed)
                .header("accept", "application/json")
                .json(&body),
        )
        .await
    }

    /// Send, then read the body a chunk at a time so the cap is a limit on what is allocated rather than a verdict on what already was.
    ///
    /// The cap applies after decompression, which is the number that matters: a small gzipped response that expands to gigabytes is exactly the case a post-hoc length check cannot catch.
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Response, HttpError> {
        let mut response = request
            .send()
            .await
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        let status = response.status();
        let final_url = response.url().to_string();

        // A body that announces itself as too large is refused before a byte of it is read.
        if let Some(declared) = response.content_length() {
            if declared > self.max_body_bytes as u64 {
                return Err(self.too_large(declared));
            }
        }

        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| HttpError::Transport(e.to_string()))?
        {
            if body.len() + chunk.len() > self.max_body_bytes {
                return Err(self.too_large((body.len() + chunk.len()) as u64));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(Response {
            status,
            // Lossy, as `text()` was with this build of reqwest: a job board that emits one bad byte should fail in the parser with a readable diagnostic, not vanish as a transport error.
            body: String::from_utf8_lossy(&body).into_owned(),
            final_url,
        })
    }

    fn too_large(&self, seen: u64) -> HttpError {
        HttpError::Transport(format!(
            "response body of at least {} bytes exceeds the {} byte cap",
            seen, self.max_body_bytes
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A one-shot HTTP server that writes exactly the bytes it is given. Reaching it needs no DNS, so the resolver guard is not in the way; the public methods that do apply the host checks are tested in `net`.
    async fn serve_raw(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut discard = [0u8; 1024];
                let _ = socket.read(&mut discard).await;
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    fn with_body(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    /// Length-less chunked transfer, which is how a body can arrive with nothing declaring its size.
    fn chunked(parts: &[&[u8]]) -> Vec<u8> {
        let mut out =
            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n"
                .to_vec();
        for part in parts {
            out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
            out.extend_from_slice(part);
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"0\r\n\r\n");
        out
    }

    fn client(max_body_bytes: usize) -> HttpClient {
        let mut client = HttpClient::new().expect("client");
        client.max_body_bytes = max_body_bytes;
        client
    }

    async fn fetch(client: &HttpClient, url: &str) -> Result<Response, HttpError> {
        client.send(client.inner.get(url)).await
    }

    #[tokio::test]
    async fn an_ordinary_response_is_read_whole() {
        let url = serve_raw(with_body(br#"{"jobs":[]}"#)).await;
        let response = fetch(&client(1024), &url).await.expect("ordinary response");
        assert_eq!(response.status.as_u16(), 200);
        assert_eq!(response.body, r#"{"jobs":[]}"#);
    }

    #[tokio::test]
    async fn a_body_of_exactly_the_cap_is_accepted() {
        let body = vec![b'x'; 64];
        let url = serve_raw(with_body(&body)).await;
        let response = fetch(&client(64), &url).await.expect("exactly at the cap");
        assert_eq!(response.body.len(), 64);
    }

    #[tokio::test]
    async fn one_byte_over_the_cap_is_refused() {
        let body = vec![b'x'; 65];
        let url = serve_raw(with_body(&body)).await;
        let error = fetch(&client(64), &url).await.expect_err("over the cap");
        assert!(
            error.to_string().contains("exceeds the 64 byte cap"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_chunked_body_is_cut_off_at_the_cap_rather_than_accumulated() {
        // Nothing declares a length here, so the only thing that can stop this is counting while reading.
        let part = vec![b'x'; 32];
        let url = serve_raw(chunked(&[&part, &part, &part, &part])).await;
        let error = fetch(&client(64), &url)
            .await
            .expect_err("a chunked body over the cap");
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[tokio::test]
    async fn a_chunked_body_within_the_cap_is_reassembled() {
        let url = serve_raw(chunked(&[b"{\"jo", b"bs\":", b"[]}"])).await;
        let response = fetch(&client(1024), &url).await.expect("chunked response");
        assert_eq!(response.body, r#"{"jobs":[]}"#);
    }

    #[tokio::test]
    async fn invalid_utf8_is_replaced_rather_than_failing_the_read() {
        // A parse error naming the payload is more useful than a transport error hiding it.
        let url = serve_raw(with_body(&[b'{', 0xff, 0xfe, b'}'])).await;
        let response = fetch(&client(1024), &url).await.expect("invalid utf-8");
        assert!(response.body.starts_with('{'), "{:?}", response.body);
        assert!(response.body.contains('\u{fffd}'), "{:?}", response.body);
    }

    #[tokio::test]
    async fn a_declared_length_over_the_cap_is_refused_before_reading() {
        let mut raw = b"HTTP/1.1 200 OK\r\ncontent-length: 1048576\r\n\r\n".to_vec();
        raw.extend_from_slice(&[b'x'; 128]);
        let url = serve_raw(raw).await;
        let error = fetch(&client(64), &url)
            .await
            .expect_err("a declared length over the cap");
        assert!(error.to_string().contains("1048576"), "{error}");
    }
}
