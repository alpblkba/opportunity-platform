use std::io;
use std::net::{IpAddr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UrlError {
    #[error("unsupported scheme {0}: only https is allowed for outbound source requests")]
    Scheme(String),
    #[error("url carries embedded credentials")]
    Credentials,
    #[error("url has no host")]
    NoHost,
    #[error("host {0} is a private, loopback or link-local address")]
    PrivateAddress(String),
    #[error("host {0} is not an allowed host for this adapter")]
    HostNotAllowed(String),
}

/// Reject anything that is not a plain https request to a public host. Applied to the URL an adapter builds and, through the redirect policy, to every redirect hop, so a source cannot steer the fetcher at the loopback interface or the cloud metadata endpoint.
pub fn check_public_https(url: &Url) -> Result<(), UrlError> {
    if url.scheme() != "https" {
        return Err(UrlError::Scheme(url.scheme().to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlError::Credentials);
    }
    match url.host() {
        None => Err(UrlError::NoHost),
        Some(Host::Ipv4(ip)) => reject_private(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => reject_private(IpAddr::V6(ip)),
        Some(Host::Domain(name)) => {
            let lowered = name.to_ascii_lowercase();
            // A name that is obviously local is refused before any lookup; a name that resolves to a private address is refused by GuardedResolver at connect time.
            let blocked = lowered == "localhost"
                || lowered.ends_with(".localhost")
                || lowered.ends_with(".local")
                || lowered.ends_with(".internal")
                || lowered.ends_with(".home.arpa");
            if blocked {
                Err(UrlError::PrivateAddress(lowered))
            } else {
                Ok(())
            }
        }
    }
}

/// Whether an address is one this tool must never connect to. Covers loopback, RFC1918, carrier-grade NAT, link-local including the cloud metadata address, broadcast, documentation ranges, IPv6 unique-local and IPv6 link-local.
pub fn is_private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.segments()[0] & 0xfe00 == 0xfc00
                || v6.segments()[0] & 0xffc0 == 0xfe80
                // An IPv4-mapped address reaches the same host as the bare IPv4 address, so it is judged by the same rules.
                || v6.to_ipv4_mapped().map(|v4| is_private_address(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

fn reject_private(ip: IpAddr) -> Result<(), UrlError> {
    if is_private_address(ip) {
        Err(UrlError::PrivateAddress(ip.to_string()))
    } else {
        Ok(())
    }
}

/// Additionally pin the host to the provider's documented API host. Adapter requests are built from a fixed template, so this catches a redirect away from the provider rather than a malformed configuration.
pub fn check_allowed_host(url: &Url, allowed: &[&str]) -> Result<(), UrlError> {
    check_public_https(url)?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if allowed.iter().any(|a| host == *a) {
        Ok(())
    } else {
        Err(UrlError::HostNotAllowed(host))
    }
}

/// DNS resolver that refuses to hand the connector a private address.
///
/// Checking a hostname and then letting the connector resolve it again leaves a window in which the answer can change, which is how DNS rebinding reaches a loopback or metadata address through a name that looked public. Doing the check inside resolution closes that window, because the addresses the connector dials are exactly the addresses this filter approved. Every redirect hop resolves through the same path, so one installation covers the whole client.
#[derive(Debug, Clone, Default)]
pub struct GuardedResolver;

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            // Port 0 because only the addresses matter here; the connector supplies the real port.
            let resolved = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| -> BoxError { Box::new(e) })?;
            let addrs: Vec<SocketAddr> = resolved.collect();
            if addrs.is_empty() {
                return Err(
                    Box::new(io::Error::other(format!("{host} did not resolve"))) as BoxError,
                );
            }
            // Refused if any answer is private, not merely if all are: a name that returns one public and one private address is the rebinding case, and picking the public one would still leave the private one reachable on a retry.
            if let Some(bad) = addrs.iter().find(|a| is_private_address(a.ip())) {
                return Err(Box::new(io::Error::other(format!(
                    "refusing to connect to {host}: it resolves to the private address {}",
                    bad.ip()
                ))) as BoxError);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Pin a destination to a provider's domain rather than a single host, for providers that serve each customer on its own subdomain.
pub fn check_host_suffix(url: &Url, suffix: &str) -> Result<(), UrlError> {
    check_public_https(url)?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == suffix || host.ends_with(&format!(".{suffix}")) {
        Ok(())
    } else {
        Err(UrlError::HostNotAllowed(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn plain_http_and_non_web_schemes_are_refused() {
        assert_eq!(
            check_public_https(&url("http://boards-api.greenhouse.io/x")),
            Err(UrlError::Scheme("http".into()))
        );
        assert_eq!(
            check_public_https(&url("file:///etc/passwd")),
            Err(UrlError::Scheme("file".into()))
        );
        assert_eq!(
            check_public_https(&url("ftp://example.org/x")),
            Err(UrlError::Scheme("ftp".into()))
        );
    }

    #[test]
    fn loopback_private_and_metadata_destinations_are_refused() {
        for candidate in [
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://10.1.2.3/x",
            "https://192.168.0.5/x",
            "https://172.16.9.9/x",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]/x",
            "https://[fd00::1]/x",
            "https://db.internal/x",
            "https://printer.local/x",
        ] {
            assert!(
                matches!(
                    check_public_https(&url(candidate)),
                    Err(UrlError::PrivateAddress(_))
                ),
                "expected {candidate} to be refused"
            );
        }
    }

    #[test]
    fn embedded_credentials_are_refused() {
        assert_eq!(
            check_public_https(&url("https://user:pass@example.org/x")),
            Err(UrlError::Credentials)
        );
    }

    #[test]
    fn host_pinning_rejects_a_redirect_off_the_provider() {
        let allowed = ["boards-api.greenhouse.io"];
        assert!(check_allowed_host(
            &url("https://boards-api.greenhouse.io/v1/boards/x/jobs"),
            &allowed
        )
        .is_ok());
        assert!(matches!(
            check_allowed_host(&url("https://attacker.example/v1/boards/x/jobs"), &allowed),
            Err(UrlError::HostNotAllowed(_))
        ));
    }

    #[test]
    fn a_provider_subdomain_is_accepted_but_a_lookalike_domain_is_not() {
        assert!(check_host_suffix(
            &url("https://acme.wd1.myworkdayjobs.com/x"),
            "myworkdayjobs.com"
        )
        .is_ok());
        assert!(
            check_host_suffix(&url("https://myworkdayjobs.com/x"), "myworkdayjobs.com").is_ok()
        );
        assert!(check_host_suffix(
            &url("https://evil-myworkdayjobs.com/x"),
            "myworkdayjobs.com"
        )
        .is_err());
        assert!(check_host_suffix(
            &url("https://myworkdayjobs.com.evil.test/x"),
            "myworkdayjobs.com"
        )
        .is_err());
    }

    #[test]
    fn ipv4_mapped_ipv6_loopback_is_refused_like_the_bare_address() {
        assert!(is_private_address("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_private_address(
            "::ffff:169.254.169.254".parse().unwrap()
        ));
        assert!(!is_private_address("::ffff:93.184.216.34".parse().unwrap()));
    }

    /// `localtest.me` is an ordinary public name whose A record is 127.0.0.1. The name check cannot see that, so this is precisely the case the resolver exists for.
    #[tokio::test]
    async fn a_public_name_that_resolves_to_loopback_passes_the_name_check_and_is_caught_by_address_filtering(
    ) {
        let candidate = url("https://localtest.me/");
        assert!(
            check_public_https(&candidate).is_ok(),
            "the name check alone cannot see the private address"
        );
        match tokio::net::lookup_host(("localtest.me", 0)).await {
            Ok(addrs) => {
                let addrs: Vec<SocketAddr> = addrs.collect();
                assert!(!addrs.is_empty());
                assert!(
                    addrs.iter().any(|a| is_private_address(a.ip())),
                    "expected localtest.me to resolve to loopback, got {addrs:?}"
                );
            }
            Err(_) => eprintln!("SKIPPED: no DNS available to resolve localtest.me"),
        }
    }

    #[test]
    fn an_ordinary_public_https_url_passes() {
        assert!(check_public_https(&url("https://api.lever.co/v0/postings/example")).is_ok());
    }
}
