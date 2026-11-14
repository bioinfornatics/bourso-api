//! Test-only routing facade. Deliberately no Deref or access to the raw client:
//! every GET/POST, including manually followed redirects, uses the same route.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use reqwest::{IntoUrl, Method, RequestBuilder, Url};

use super::BoursoWebClient;

pub(super) struct Client {
    inner: reqwest::Client,
    endpoint: Option<Url>,
}

impl From<reqwest::Client> for Client {
    fn from(inner: reqwest::Client) -> Self {
        Self {
            inner,
            endpoint: None,
        }
    }
}

impl Client {
    pub(super) fn route_url(&self, original: Url) -> Url {
        match &self.endpoint {
            None => original,
            Some(endpoint) => {
                // Copy path/query, not a joined URL: //host paths, absolute
                // redirects and changing Config.api_url cannot replace the origin.
                let mut local = endpoint.clone();
                local.set_path(original.path());
                local.set_query(original.query());
                local.set_fragment(original.fragment());
                local
            }
        }
    }

    fn request(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        match url.into_url() {
            Ok(url) => self.inner.request(method, self.route_url(url)),
            // A malformed destination must remain a builder error, never fall
            // back to an external URL (nor silently issue a request to /).
            Err(_) => self.inner.request(method, ""),
        }
    }

    pub(super) fn get(&self, url: impl IntoUrl) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    pub(super) fn post(&self, url: impl IntoUrl) -> RequestBuilder {
        self.request(Method::POST, url)
    }
}

impl BoursoWebClient {
    /// Available only to crate unit tests. No hostnames, wildcard addresses,
    /// external IPs, environment-derived endpoints or external fallback.
    pub(crate) fn for_test(address: SocketAddr, timeout: Duration) -> anyhow::Result<Self> {
        anyhow::ensure!(address.ip().is_loopback(), "test server must be loopback");
        anyhow::ensure!(address.port() != 0, "test server must have a bound port");
        let mut client = Self::new();
        client.client = Client {
            inner: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .cookie_provider(Arc::clone(&client.cookie_store))
                .timeout(timeout)
                .build()?,
            endpoint: Some(Url::parse(&format!("http://{address}/"))?),
        };
        Ok(client)
    }
}
