//! The one place every source client (GitHub, Wago.io, TukUI, WoWInterface)
//! is configured, so none of them can wait forever on a stalled server.

use std::time::Duration;

use reqwest::ClientBuilder;

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
}
