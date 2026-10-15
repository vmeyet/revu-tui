use std::sync::Once;
use std::time::Duration;

/// A host that has not taken the connection by then is down or out of reach.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Not one byte for this long on an open request: the link died under it, as after a sleep or a VPN drop.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How often an idle connection is probed, so a dead one is found before the next request trusts it.
const KEEPALIVE: Duration = Duration::from_secs(15);

/// The one way to start a `reqwest` client: its TLS runs on ring, the only crypto provider revu links,
/// which rustls needs installed before the first client is built. No request waits forever on a
/// silent host; a client that knows how long its answers take adds its own total limit.
pub fn client() -> reqwest::ClientBuilder {
    static RING: Once = Once::new();
    RING.call_once(|| {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
    });
    #[expect(clippy::disallowed_methods, reason = "this is the one place that builds a client, after installing the TLS provider")]
    reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT).read_timeout(READ_TIMEOUT).tcp_keepalive(KEEPALIVE)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn builds_a_tls_client_more_than_once() {
        client().build().unwrap();
        client().build().unwrap();
    }

    /// A server that takes every connection and never answers, as a host does once the link to it died.
    pub(crate) async fn silent_server() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = vec![];
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        format!("http://{address}/")
    }

    #[tokio::test]
    async fn a_host_that_never_answers_is_given_up_on() {
        let url = silent_server().await;
        let client = client().read_timeout(Duration::from_millis(200)).build().unwrap();
        let err = tokio::time::timeout(Duration::from_secs(5), client.get(url).send()).await.expect("the request ended").unwrap_err();
        assert!(err.is_timeout(), "{err}");
    }
}
