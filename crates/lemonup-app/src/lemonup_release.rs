//! Asks GitHub for LemonUp's own newest release, for the TUI to say when a newer LemonUp is
//! out. It uses the GitHub client every addon check uses (its token, User-Agent and timeouts),
//! and the answer is kept by `lemonup_core::lemonup_release`.

use std::path::Path;

use lemonup_core::lemonup_release::{FetchedRelease, LatestRelease, record_check};
use reqwest::Client;
use serde::Deserialize;
use time::OffsetDateTime;

use crate::github::{build_github_client, ensure_github_success};
use crate::source_error::SourceError;

/// LemonUp's newest release. GitHub never names a draft or a prerelease here.
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/archcorsair/lemonup/releases/latest";

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    #[serde(with = "time::serde::rfc3339")]
    published_at: OffsetDateTime,
}

/// Looks for LemonUp's newest release and keeps the answer in `cache`. A failure keeps the
/// answer already there and only moves when it was last checked.
pub(crate) async fn check_latest_release(cache: &Path) -> Result<LatestRelease, SourceError> {
    let client = build_github_client().map_err(SourceError::from)?;
    check_latest_release_at(
        &client,
        LATEST_RELEASE_URL,
        cache,
        OffsetDateTime::now_utc(),
    )
    .await
}

async fn check_latest_release_at(
    client: &Client,
    url: &str,
    cache: &Path,
    now: OffsetDateTime,
) -> Result<LatestRelease, SourceError> {
    record_check(cache, fetch_latest_release(client, url).await, now)
}

async fn fetch_latest_release(client: &Client, url: &str) -> Result<FetchedRelease, SourceError> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(SourceError::from_reqwest)?;
    let release = ensure_github_success(response)?
        .json::<ReleaseResponse>()
        .await
        .map_err(SourceError::from_reqwest)?;
    Ok(FetchedRelease {
        tag: release.tag_name,
        published_at: release.published_at,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;

    use lemonup_core::lemonup_release::{read_latest_release, write_latest_release};

    use super::*;
    use crate::source_error::SourceErrorKind;

    /// A trimmed `releases/latest` body as GitHub sends it.
    const SAMPLE: &str = r#"{
        "url": "https://api.github.com/repos/archcorsair/lemonup/releases/1",
        "tag_name": "v2.8.1",
        "name": "2.8.1",
        "draft": false,
        "prerelease": false,
        "created_at": "2026-11-19T11:58:02Z",
        "published_at": "2026-11-19T12:00:00Z",
        "assets": []
    }"#;

    fn at(rfc3339: &str) -> OffsetDateTime {
        OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
            .expect("time")
    }

    /// Answers one request on localhost with `response`, and returns its URL.
    fn serve_once(response: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(response.as_bytes());
        });
        format!("http://{address}/repos/archcorsair/lemonup/releases/latest")
    }

    fn http(status: &str, headers: &[&str], body: &str) -> String {
        let mut response = format!("HTTP/1.1 {status}\r\n");
        for header in headers {
            response.push_str(header);
            response.push_str("\r\n");
        }
        response.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        response
    }

    fn client() -> Client {
        crate::http::client_builder().build().expect("client")
    }

    /// A cache holding an older answer, and that answer.
    fn planted_cache() -> (tempfile::TempDir, PathBuf, LatestRelease) {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("latest-release.json");
        let old = LatestRelease {
            version: "2.7.5".to_string(),
            published_at: at("2026-11-01T00:00:00Z"),
            checked_at: at("2026-11-10T00:00:00Z"),
        };
        write_latest_release(&path, &old).expect("plant");
        (temp, path, old)
    }

    /// After a failed look at `now`: the old answer, with only `checked_at` moved.
    fn assert_kept(cache: &Path, old: &LatestRelease, now: OffsetDateTime, case: &str) {
        let kept = read_latest_release(cache).expect("kept");
        assert_eq!(
            kept,
            LatestRelease {
                checked_at: now,
                ..old.clone()
            },
            "{case}"
        );
    }

    #[test]
    fn a_releases_latest_body_gives_the_tag_and_when_it_was_published() {
        let release: ReleaseResponse = serde_json::from_str(SAMPLE).expect("parse");
        assert_eq!(release.tag_name, "v2.8.1");
        assert_eq!(release.published_at, at("2026-11-19T12:00:00Z"));
    }

    #[tokio::test]
    async fn an_answer_replaces_the_cache() {
        let (_temp, cache, _) = planted_cache();
        let url = serve_once(http("200 OK", &[], SAMPLE));
        let now = at("2026-11-21T09:00:00Z");
        let latest = check_latest_release_at(&client(), &url, &cache, now)
            .await
            .expect("answer");
        let expected = LatestRelease {
            version: "2.8.1".to_string(),
            published_at: at("2026-11-19T12:00:00Z"),
            checked_at: now,
        };
        assert_eq!(latest, expected);
        assert_eq!(read_latest_release(&cache), Some(expected));
    }

    #[tokio::test]
    async fn failures_are_source_errors_and_keep_the_cache() {
        let rate_limited = http(
            "403 Forbidden",
            &["x-ratelimit-remaining: 0", "x-ratelimit-reset: 1700000000"],
            "{\"message\": \"API rate limit exceeded\"}",
        );
        for (response, kind) in [
            (rate_limited, SourceErrorKind::SlowDown),
            (
                http("500 Internal Server Error", &[], "{}"),
                SourceErrorKind::Other,
            ),
            (http("404 Not Found", &[], "{}"), SourceErrorKind::NotFound),
            (http("200 OK", &[], "<html>"), SourceErrorKind::Other),
            (
                http("200 OK", &[], "{\"tag_name\": \"v2.8.1\"}"),
                SourceErrorKind::Other,
            ),
        ] {
            let (_temp, cache, old) = planted_cache();
            let url = serve_once(response.clone());
            let now = at("2026-11-21T09:00:00Z");
            let error = check_latest_release_at(&client(), &url, &cache, now)
                .await
                .expect_err("a failure");
            assert_eq!(error.kind, kind, "{response}");
            assert_kept(&cache, &old, now, &response);
        }
    }

    #[tokio::test]
    async fn offline_is_no_connection_and_keeps_the_cache() {
        let (_temp, cache, old) = planted_cache();
        let now = at("2026-11-21T09:00:00Z");
        // Port 9 on localhost: nothing listens, so the connect is refused.
        let error = check_latest_release_at(&client(), "http://127.0.0.1:9/", &cache, now)
            .await
            .expect_err("offline");
        assert_eq!(error.kind, SourceErrorKind::NoConnection);
        assert_kept(&cache, &old, now, "offline");
    }

    #[tokio::test]
    async fn a_missing_cache_stays_missing_after_a_failure() {
        let temp = tempfile::tempdir().expect("temp");
        let cache = temp.path().join("latest-release.json");
        let url = serve_once(http("502 Bad Gateway", &[], ""));
        check_latest_release_at(&client(), &url, &cache, OffsetDateTime::now_utc())
            .await
            .expect_err("a failure");
        assert!(!cache.exists());
    }
}
