//! Outbound-fetch SSRF guard.
//!
//! Feed / art / episode-media URLs originate in (user-supplied) RSS feeds, so we
//! reject hosts that resolve to a non-public address (loopback, private, link-local,
//! CGNAT, cloud-metadata, …) to stop requests being aimed at internal services.
//!
//! Enforcement is a custom reqwest DNS resolver ([`PublicOnlyResolver`]) installed on
//! the feed / download / art clients. reqwest re-resolves **every** connection through
//! it — including each redirect hop — so the whole request chain is covered, not just
//! the initial URL.
//!
//! The policy is a process global, OFF (allow everything) until [`configure`] flips
//! it — which only `main` does, from `cfg.allow_private_network`. Unit tests and the
//! in-process integ/e2e harness (which build the router directly, never running
//! `main`) keep fetching their `127.0.0.1` mock servers freely.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// `true` = block non-public hosts. Default `false` (allow) so non-production
/// callers (unit tests, the in-process integ/e2e server) are unaffected.
static BLOCK_PRIVATE: AtomicBool = AtomicBool::new(false);

/// `server_fetch_user_agent` when the deployer set one. `RwLock`, not `OnceLock`:
/// the embedded supervisor re-configures on every in-process restart.
static USER_AGENT_OVERRIDE: RwLock<Option<String>> = RwLock::new(None);

/// Set the outbound User-Agent for every client. Call once at startup, beside
/// [`configure`]. `None`/blank keeps each client's purpose-tagged default.
pub fn configure_user_agent(user_agent: Option<String>) {
    let value = user_agent.filter(|ua| !ua.trim().is_empty());
    *USER_AGENT_OVERRIDE
        .write()
        .unwrap_or_else(|p| p.into_inner()) = value;
}

/// The User-Agent an outbound client should send: the configured override when
/// set, else `Halogen/<version> (+<purpose>)`.
///
/// NOT cosmetic — media CDNs behind Cloudflare bot management (Buzzsprout,
/// verified) 403 a request with no User-Agent or a known-automation one. Any
/// honest self-identifying UA passes: podcast hosts want real clients to fetch
/// (that's their download analytics), they block anonymous scrapers.
pub fn user_agent(purpose: &str) -> String {
    if let Some(ua) = USER_AGENT_OVERRIDE
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        return ua;
    }
    format!("Halogen/{} (+{purpose})", env!("CARGO_PKG_VERSION"))
}

/// Set the outbound-fetch policy. Call once at startup. `allow_private == true`
/// disables the guard (dev, or a feed host on a private LAN); the production
/// default is `false`, which blocks non-public addresses.
pub fn configure(allow_private: bool) {
    BLOCK_PRIVATE.store(!allow_private, Ordering::Relaxed);
}

/// A reqwest DNS resolver that rejects any host resolving to a non-public address
/// while the guard is on. Install it on a client and every connection it makes —
/// initial request and each redirect hop — is checked.
pub struct PublicOnlyResolver;

/// The shared resolver for outbound (feed / download / art) clients.
pub fn dns_resolver() -> Arc<PublicOnlyResolver> {
    Arc::new(PublicOnlyResolver)
}

/// A `reqwest::ClientBuilder` pre-wired with the SSRF DNS guard. EVERY outbound
/// client (feed, download, art, discover) should start here and then layer on its
/// own timeouts / redirect policy / gzip / user-agent, so the guard is installed
/// in exactly one place and can't be forgotten — which it had been on the discover
/// client, the one hole this closes.
///
/// A default `connect_timeout` is applied here too so a slow/hostile host can't
/// stall the TCP+TLS handshake on any client — it had been forgotten on the feed
/// and chapters clients. Callers may still override it; it bounds only connection
/// establishment, not the (per-client) total/read timeout.
pub fn guarded_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .dns_resolver(dns_resolver())
        .connect_timeout(std::time::Duration::from_secs(5))
}

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            // Port is irrelevant for the address check; reqwest sets the real one.
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if BLOCK_PRIVATE.load(Ordering::Relaxed)
                && let Some(bad) = addrs.iter().find(|a| is_blocked_ip(a.ip()))
            {
                return Err(
                    format!("blocked non-public address {} for host {host}", bad.ip()).into(),
                );
            }
            let iter: Addrs = Box::new(addrs.into_iter());
            Ok(iter)
        })
    }
}

/// Non-public / SSRF-sensitive ranges, using only stable `std` APIs.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_private()            // 10/8, 172.16/12, 192.168/16
                || a.is_loopback()    // 127/8
                || a.is_link_local()  // 169.254/16 (incl. cloud metadata 169.254.169.254)
                || a.is_broadcast()
                || a.is_documentation()
                || a.is_multicast()   // 224.0.0.0/4 (parity with the V6 arm)
                || a.is_unspecified() // 0.0.0.0
                || o[0] == 0                              // 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64)   // 100.64.0.0/10 CGNAT
                || o[0] >= 240 // 240.0.0.0/4 reserved
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00  // fc00::/7  unique-local
                || (s[0] & 0xffc0) == 0xfe80  // fe80::/10 link-local
                || a
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_blocked_ip(IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests;
