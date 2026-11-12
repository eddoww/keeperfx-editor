//! The wire: a [`Transport`] makes single `GET` requests (no redirects followed), and a
//! [`UrlPolicy`] says which URLs may be contacted at all. Redirects are followed by the client
//! ([`crate::client::Client`]), which checks every hop against the policy.

use crate::{Error, Result};
use std::io::Read;
use std::time::Duration;

/// GitHub's hosts that serve the release list and the release files. `github.com` answers a
/// download with a redirect to `release-assets.githubusercontent.com` (seen on the live site,
/// 2026-11); `objects.githubusercontent.com` served release files before that and is kept for
/// older links.
pub const GITHUB_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One `GET` request. HTTP error statuses and redirects are returned as a [`Response`], not
/// followed and not turned into errors. Tests substitute their own.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, String)]) -> Result<Response>;
}

/// Which URLs may be contacted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlPolicy {
    /// Hosts reachable over HTTPS.
    pub https_hosts: Vec<String>,
    /// Hosts (with port, `127.0.0.1:8080`) reachable over plain HTTP: only ever set by tests,
    /// which serve a mock GitHub on the loopback interface.
    pub http_hosts: Vec<String>,
}

impl UrlPolicy {
    /// HTTPS to [`GITHUB_HOSTS`] only.
    pub fn github() -> Self {
        UrlPolicy {
            https_hosts: GITHUB_HOSTS.iter().map(|s| s.to_string()).collect(),
            http_hosts: vec![],
        }
    }

    /// Plain HTTP to one local test server (`127.0.0.1:port`), nothing else.
    pub fn local_test_server(authority: &str) -> Self {
        UrlPolicy {
            https_hosts: vec![],
            http_hosts: vec![authority.to_string()],
        }
    }

    /// The URL may be contacted.
    pub fn check(&self, url: &str) -> Result<()> {
        let refuse = || Error::UrlNotAllowed(url.to_string());
        let (scheme, authority) = split_url(url).ok_or_else(refuse)?;
        let ok = match scheme {
            "https" => {
                // no explicit port other than 443, no user info
                let host = authority.strip_suffix(":443").unwrap_or(authority);
                self.https_hosts
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(host))
            }
            "http" => self
                .http_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(authority)),
            _ => false,
        };
        if ok {
            Ok(())
        } else {
            Err(refuse())
        }
    }
}

/// Scheme and authority (`host[:port]`) of an absolute URL; `None` for anything unusual (user
/// info, missing host, odd characters).
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty()
        || !authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':')
    {
        return None;
    }
    Some((scheme, authority))
}

/// Resolve a redirect target against the URL that answered: absolute URLs as they are, an
/// absolute path on the same scheme and host. Anything else is refused.
pub fn resolve_location(base: &str, location: &str) -> Option<String> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_string());
    }
    if location.starts_with('/') && !location.starts_with("//") {
        let (scheme, authority) = split_url(base)?;
        return Some(format!("{scheme}://{authority}{location}"));
    }
    None
}

/// The real wire: `ureq` with rustls (as the Workshop client), no automatic redirects.
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .redirects(0)
            .https_only(false) // the policy decides; tests use plain HTTP on the loopback
            .build();
        HttpTransport { agent }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str, headers: &[(&str, String)]) -> Result<Response> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => {
                return Err(Error::Network {
                    url: url.to_string(),
                    message: e.to_string(),
                })
            }
        };
        let headers = resp
            .headers_names()
            .into_iter()
            .filter_map(|n| resp.header(&n).map(|v| (n.clone(), v.to_string())))
            .collect();
        let status = resp.status();
        Ok(Response {
            status,
            headers,
            body: Box::new(resp.into_reader()),
        })
    }
}
