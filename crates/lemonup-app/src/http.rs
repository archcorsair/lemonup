//! The one place every source client (GitHub, Wago.io, TukUI, WoWInterface)
//! is configured, so none of them can wait forever on a stalled server.

use std::time::Duration;

use reqwest::header::{HeaderValue, LOCATION};
use reqwest::{ClientBuilder, Response, StatusCode, Url};

use crate::source_error::{SourceError, SourceErrorKind};

/// How long to wait for a connection to be established.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the next chunk of data. This is per read, not a total
/// deadline, so a large download still works as long as bytes keep flowing.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn client_builder() -> ClientBuilder {
    client_builder_with(CONNECT_TIMEOUT, READ_TIMEOUT)
}

fn client_builder_with(connect: Duration, read: Duration) -> ClientBuilder {
    reqwest::Client::builder()
        .user_agent("LemonUp/2 (+https://github.com/archcorsair/lemonup)")
        .connect_timeout(connect)
        .read_timeout(read)
}

/// How many redirects `follow_download_redirects` follows itself.
const MAX_FOLLOWED_REDIRECTS: usize = 5;

/// The download `response` once every redirect is followed, so a redirect page is never taken
/// for the file.
///
/// reqwest follows a redirect only when its `Location` is a valid address. Wago.io's file
/// server sends one with raw spaces (`omnicd-v2.8.35 (1).zip`), so reqwest hands back the 3xx
/// with its small "Redirecting to..." page. This reads `Location` the way a browser does, which
/// encodes the spaces, and fetches it with a new client, so no key or token goes to the new
/// address. A redirect still left after that is an error.
pub(crate) async fn follow_download_redirects(
    mut response: Response,
    map_error: fn(reqwest::Error) -> SourceError,
) -> Result<Response, SourceError> {
    let mut client = None;
    for _ in 0..MAX_FOLLOWED_REDIRECTS {
        if !response.status().is_redirection() {
            return checked_after_redirect(response, client.is_some());
        }
        let Some(next) = redirect_target(response.url(), response.headers().get(LOCATION)) else {
            return Err(stopped_at_redirect(response.status()));
        };
        let client = match &client {
            Some(client) => client,
            None => client.insert(client_builder().build().map_err(map_error)?),
        };
        response = client.get(next).send().await.map_err(map_error)?;
    }
    if response.status().is_redirection() {
        return Err(stopped_at_redirect(response.status()));
    }
    checked_after_redirect(response, true)
}

/// A 401 or 403 from the address LemonUp followed itself can't be about the key or token,
/// which was never sent there, so it is not left to the source to call a rejected key.
fn checked_after_redirect(response: Response, followed: bool) -> Result<Response, SourceError> {
    let status = response.status();
    if followed && matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return Err(SourceError::new(
            SourceErrorKind::Other,
            format!("the file server refused the download ({status})"),
        ));
    }
    Ok(response)
}

/// Where a redirect from `current` points, resolved and encoded the way a browser does. `None`
/// when there is no usable address, or when it would leave HTTPS.
fn redirect_target(current: &Url, location: Option<&HeaderValue>) -> Option<Url> {
    let location = std::str::from_utf8(location?.as_bytes()).ok()?;
    let next = current.join(location).ok()?;
    let allowed = match next.scheme() {
        "https" => true,
        "http" => current.scheme() == "http",
        _ => false,
    };
    allowed.then_some(next)
}

fn stopped_at_redirect(status: StatusCode) -> SourceError {
    SourceError::new(
        SourceErrorKind::Other,
        format!("the download stopped at a redirect ({status}) instead of the file"),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::source_error::{SourceError, SourceErrorKind};

    #[test]
    fn the_shared_builder_builds() {
        client_builder().build().expect("client builds");
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out_as_no_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        // Accept the connection and hold it open without ever responding.
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(socket);
        });

        let client = client_builder_with(Duration::from_secs(5), Duration::from_millis(200))
            .build()
            .expect("client builds");
        let started = Instant::now();
        let error = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect_err("the server never answers");

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(error.is_timeout());
        assert_eq!(
            SourceError::from_reqwest(error).kind,
            SourceErrorKind::NoConnection
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_redirect_to_an_address_with_a_raw_space_is_followed_without_the_key() {
        let server = test_server::serve(vec![
            test_server::redirect("/files/omnicd-v2.8.35 (1).zip"),
            test_server::ok(b"PK\x03\x04 the zip"),
        ]);
        let client = client_builder().build().expect("client builds");
        let response = client
            .get(server.url("/download/omnicd"))
            .bearer_auth("secret-key")
            .send()
            .await
            .expect("send");
        // reqwest can't read the address, so it hands back the redirect page.
        assert_eq!(response.status(), StatusCode::FOUND);

        let response = follow_download_redirects(response, SourceError::from_reqwest)
            .await
            .expect("followed");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.bytes().await.expect("body").as_ref(),
            b"PK\x03\x04 the zip"
        );

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /download/omnicd HTTP/1.1\r\n"));
        assert!(test_server::has_header(&requests[0], "authorization"));
        assert!(requests[1].starts_with("GET /files/omnicd-v2.8.35%20(1).zip HTTP/1.1\r\n"));
        assert!(!test_server::has_header(&requests[1], "authorization"));
    }

    #[tokio::test]
    async fn a_redirect_without_an_address_is_an_error_not_the_file() {
        let server = test_server::serve(vec![test_server::response(
            "302 Found",
            &[],
            b"Redirecting to...",
        )]);
        let client = client_builder().build().expect("client builds");
        let response = client
            .get(server.url("/download"))
            .send()
            .await
            .expect("send");

        let error = follow_download_redirects(response, SourceError::from_reqwest)
            .await
            .expect_err("a redirect page is not the file");
        assert_eq!(error.kind, SourceErrorKind::Other);
        assert!(error.detail.contains("redirect"), "{}", error.detail);
    }

    #[tokio::test]
    async fn a_refusal_from_the_followed_address_is_not_a_rejected_key() {
        let server = test_server::serve(vec![
            test_server::redirect("/files/a b.zip"),
            test_server::response("403 Forbidden", &[], b"expired"),
        ]);
        let client = client_builder().build().expect("client builds");
        let response = client
            .get(server.url("/download"))
            .bearer_auth("secret-key")
            .send()
            .await
            .expect("send");

        let error = follow_download_redirects(response, SourceError::from_reqwest)
            .await
            .expect_err("refused");
        assert_eq!(error.kind, SourceErrorKind::Other);
        assert!(error.detail.contains("403"), "{}", error.detail);
    }

    #[tokio::test]
    async fn redirects_that_never_end_are_an_error() {
        let server = test_server::serve(
            (0..=MAX_FOLLOWED_REDIRECTS)
                .map(|_| test_server::redirect("/again and again"))
                .collect(),
        );
        let client = client_builder().build().expect("client builds");
        let response = client
            .get(server.url("/download"))
            .send()
            .await
            .expect("send");

        let error = follow_download_redirects(response, SourceError::from_reqwest)
            .await
            .expect_err("never reaches a file");
        assert!(error.detail.contains("redirect"), "{}", error.detail);
        assert_eq!(server.requests().len(), MAX_FOLLOWED_REDIRECTS + 1);
    }

    #[test]
    fn a_redirect_address_is_resolved_like_a_browser_and_never_leaves_https() {
        let https = Url::parse("https://addons.wago.io/external/download/1?x=1").expect("url");
        let target = |location: &str| {
            redirect_target(
                &https,
                Some(&HeaderValue::from_str(location).expect("header")),
            )
            .map(String::from)
        };
        assert_eq!(
            target("https://cdn.wago.io/a/omnicd-v2.8.35 (1).zip").as_deref(),
            Some("https://cdn.wago.io/a/omnicd-v2.8.35%20(1).zip")
        );
        assert_eq!(
            target("/files/a b.zip").as_deref(),
            Some("https://addons.wago.io/files/a%20b.zip")
        );
        assert_eq!(target("http://cdn.wago.io/a.zip"), None);
        assert_eq!(target("ftp://cdn.wago.io/a.zip"), None);
        assert_eq!(redirect_target(&https, None), None);

        let http = Url::parse("http://127.0.0.1:1/download").expect("url");
        assert_eq!(
            redirect_target(
                &http,
                Some(&HeaderValue::from_static("http://127.0.0.1:1/a.zip"))
            )
            .map(String::from)
            .as_deref(),
            Some("http://127.0.0.1:1/a.zip")
        );
    }
}

/// A server on localhost that answers each connection with the next of a list of responses
/// and keeps the requests it got, for download tests.
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::sync::{Arc, Mutex};

    pub(crate) struct TestServer {
        address: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl TestServer {
        pub(crate) fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.address)
        }

        /// The requests answered so far, head only.
        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("requests").clone()
        }
    }

    pub(crate) fn serve(responses: Vec<Vec<u8>>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&requests);
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                }
                kept.lock()
                    .expect("requests")
                    .push(String::from_utf8_lossy(&request).into_owned());
                let _ = stream.write_all(&response);
            }
        });
        TestServer { address, requests }
    }

    pub(crate) fn response(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {status}\r\n");
        for header in headers {
            head.push_str(header);
            head.push_str("\r\n");
        }
        head.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        ));
        let mut response = head.into_bytes();
        response.extend_from_slice(body);
        response
    }

    /// A 302 to `location` on the same server, sent exactly as given (spaces and all), with
    /// the small page Wago.io's file server sends along.
    pub(crate) fn redirect(location: &str) -> Vec<u8> {
        response(
            "302 Found",
            &[
                &format!("Location: {location}"),
                "Content-Type: text/html; charset=utf-8",
            ],
            format!("Redirecting to {location}").as_bytes(),
        )
    }

    pub(crate) fn ok(body: &[u8]) -> Vec<u8> {
        response("200 OK", &["Content-Type: application/zip"], body)
    }

    pub(crate) fn has_header(request: &str, name: &str) -> bool {
        request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .any(|(header, _)| header.trim().eq_ignore_ascii_case(name))
    }
}
