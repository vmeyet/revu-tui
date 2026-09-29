use std::sync::Once;

/// The one way to start a `reqwest` client: its TLS runs on ring, the only crypto provider revu links,
/// which rustls needs installed before the first client is built.
pub fn client() -> reqwest::ClientBuilder {
    static RING: Once = Once::new();
    RING.call_once(|| {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
    });
    #[allow(clippy::disallowed_methods)]
    reqwest::Client::builder()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn builds_a_tls_client_more_than_once() {
        client().build().unwrap();
        client().build().unwrap();
    }
}
