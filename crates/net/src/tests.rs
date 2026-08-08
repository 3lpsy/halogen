use super::is_blocked_ip;
use std::net::IpAddr;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// Regression (M6 SSRF): the classifier the DNS resolver consults must block
/// every internal/SSRF-sensitive range and allow genuinely public addresses —
/// including the range boundaries.
#[test]
fn ssrf_classifier_blocks_internal_allows_public() {
    for s in [
        "127.0.0.1",
        "127.10.20.30",
        "10.0.0.5",
        "172.16.3.1",
        "172.31.255.255", // top of 172.16/12
        "192.168.1.1",
        "169.254.169.254", // cloud metadata
        "100.64.0.1",
        "100.127.255.255", // top of 100.64/10 CGNAT
        "224.0.0.1",       // IPv4 multicast (224.0.0.0/4)
        "239.255.255.255", // top of IPv4 multicast
        "0.0.0.0",
        "::1",
        "fe80::1",
        "fc00::1",
        "::ffff:10.0.0.1",  // v4-mapped private
        "::ffff:127.0.0.1", // v4-mapped loopback
    ] {
        assert!(is_blocked_ip(ip(s)), "{s} must be blocked (SSRF)");
    }
    for s in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "172.32.0.1",     // just outside 172.16/12
        "100.63.255.255", // just below 100.64/10
        "2606:4700:4700::1111",
    ] {
        assert!(!is_blocked_ip(ip(s)), "{s} must be allowed (public)");
    }
}

/// Unconfigured: each client gets its own purpose-tagged identity. The exact
/// shape matters — an absent or anonymous UA is what media CDNs 403.
#[test]
fn user_agent_defaults_are_purpose_tagged() {
    let ua = super::user_agent("podcast-download");
    assert!(
        ua.starts_with("Halogen/") && ua.ends_with(" (+podcast-download)"),
        "unexpected default user-agent: {ua}"
    );
    assert_ne!(
        super::user_agent("podcast-feed"),
        ua,
        "each purpose identifies itself distinctly"
    );
}

/// `server_fetch_user_agent` replaces the default for EVERY client, and a blank
/// value is treated as unset rather than sending an empty UA (which is exactly
/// what gets blocked).
#[test]
fn configured_user_agent_overrides_every_purpose() {
    super::configure_user_agent(Some("MyPodcatcher/2.0".to_string()));
    assert_eq!(super::user_agent("podcast-download"), "MyPodcatcher/2.0");
    assert_eq!(super::user_agent("podcast-art"), "MyPodcatcher/2.0");

    super::configure_user_agent(Some("   ".to_string()));
    assert!(
        super::user_agent("podcast-feed").starts_with("Halogen/"),
        "a blank override must fall back to the default"
    );

    super::configure_user_agent(None);
    assert!(super::user_agent("podcast-feed").starts_with("Halogen/"));
}
