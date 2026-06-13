#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use quaid::core::conversation::slm::LazySlmRunner;
use quaid::mcp::http::{
    bind_with_token_guard, build_connection_service, is_loopback_host, run_http,
    validate_loopback_request_head, HttpConfig, HttpConfigError, DEFAULT_HTTP_BIND,
    DEFAULT_HTTP_PORT,
};

#[test]
fn default_config_is_loopback_but_untrusted() {
    let cfg = HttpConfig::default();

    assert_eq!(cfg.port, DEFAULT_HTTP_PORT);
    assert_eq!(cfg.bind, DEFAULT_HTTP_BIND);
    assert!(cfg.token_file.is_none());
    assert!(!cfg.trusted_loopback);
    assert!(cfg.is_loopback());
}

#[test]
fn loopback_trusted_no_token_is_accepted() {
    let cfg = HttpConfig {
        port: 4010,
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        token_file: None,
        trusted_loopback: true,
    };

    let addr = bind_with_token_guard(&cfg).unwrap();

    assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(addr.port(), 4010);
}

#[test]
fn loopback_untrusted_no_token_fails_closed() {
    let cfg = HttpConfig::default();

    let error = bind_with_token_guard(&cfg).unwrap_err();

    assert!(matches!(error, HttpConfigError::LoopbackUntrustedNoToken));
}

#[test]
fn non_loopback_bind_fails_closed_before_token_policy() {
    let cfg = HttpConfig {
        port: 4010,
        bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        token_file: Some("/tmp/quaid-token".into()),
        trusted_loopback: true,
    };

    let error = bind_with_token_guard(&cfg).unwrap_err();

    assert!(matches!(
        error,
        HttpConfigError::NonLoopbackBindUnsupported { bind }
            if bind == IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    ));
}

#[test]
fn sse_connections_share_one_slm_runner() {
    // The HTTP transport hoists one process-wide LazySlmRunner and clones the
    // Arc into every per-connection QuaidServer. Two connections built from the
    // same shared runner must observe the same runner instance, while a server
    // built from an independent runner must not.
    let shared = Arc::new(LazySlmRunner::new());
    let factory = || rusqlite::Connection::open_in_memory().map_err(anyhow::Error::from);

    let server_a = build_connection_service(&factory, &shared);
    let server_b = build_connection_service(&factory, &shared);
    assert_eq!(
        server_a.slm_identity(),
        server_b.slm_identity(),
        "two SSE connections must share one SLM runner instance"
    );

    let other = Arc::new(LazySlmRunner::new());
    let server_c = build_connection_service(&factory, &other);
    assert_ne!(
        server_a.slm_identity(),
        server_c.slm_identity(),
        "a server built from a different runner must not alias the shared one"
    );
}

#[tokio::test]
async fn run_http_rejects_invalid_config_before_opening_database() {
    let opened = Arc::new(AtomicBool::new(false));
    let opened_for_factory = Arc::clone(&opened);

    let result = run_http(
        move || {
            opened_for_factory.store(true, Ordering::SeqCst);
            rusqlite::Connection::open_in_memory().map_err(anyhow::Error::from)
        },
        HttpConfig::default(),
    )
    .await;

    let error = result.unwrap_err();
    assert!(matches!(
        error.downcast_ref::<HttpConfigError>(),
        Some(HttpConfigError::LoopbackUntrustedNoToken)
    ));
    assert!(!opened.load(Ordering::SeqCst));
}

// ── Security hardening: Origin/Host validation (DNS-rebinding) ─────

fn head_with(headers: &[&str]) -> String {
    let mut head = String::from("GET /sse HTTP/1.1\r\n");
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    head
}

#[test]
fn request_with_loopback_host_and_no_origin_is_accepted() {
    let head = head_with(&["Host: 127.0.0.1:3112"]);
    assert!(validate_loopback_request_head(&head).is_ok());
}

#[test]
fn request_with_localhost_host_is_accepted() {
    assert!(validate_loopback_request_head(&head_with(&["Host: localhost:3112"])).is_ok());
    assert!(validate_loopback_request_head(&head_with(&["Host: [::1]:3112"])).is_ok());
}

#[test]
fn request_with_loopback_origin_is_accepted() {
    let head = head_with(&[
        "Host: 127.0.0.1:3112",
        "Origin: http://127.0.0.1:3112",
        "Referer: http://localhost:3112/app",
    ]);
    assert!(validate_loopback_request_head(&head).is_ok());
}

#[test]
fn request_with_non_loopback_host_is_rejected() {
    // The hallmark of DNS-rebinding: a rebound hostname still arrives in Host.
    let head = head_with(&["Host: attacker.example.com"]);
    let error = validate_loopback_request_head(&head).unwrap_err();
    assert!(error.contains("Host"), "error={error}");
}

#[test]
fn request_with_non_loopback_origin_is_rejected() {
    let head = head_with(&["Host: 127.0.0.1:3112", "Origin: http://evil.example.com"]);
    let error = validate_loopback_request_head(&head).unwrap_err();
    assert!(error.contains("Origin"), "error={error}");
}

#[test]
fn request_with_non_loopback_referer_is_rejected() {
    let head = head_with(&[
        "Host: 127.0.0.1:3112",
        "Referer: https://evil.example.com/page",
    ]);
    let error = validate_loopback_request_head(&head).unwrap_err();
    assert!(error.contains("Referer"), "error={error}");
}

#[test]
fn request_missing_host_header_is_rejected() {
    let head = head_with(&["Origin: http://127.0.0.1:3112"]);
    assert!(validate_loopback_request_head(&head).is_err());
}

#[test]
fn origin_with_userinfo_smuggling_a_loopback_label_is_rejected() {
    let head = head_with(&[
        "Host: 127.0.0.1:3112",
        "Origin: http://127.0.0.1@evil.example.com",
    ]);
    assert!(validate_loopback_request_head(&head).is_err());
}

#[test]
fn is_loopback_host_classifies_common_values() {
    assert!(is_loopback_host("127.0.0.1"));
    assert!(is_loopback_host("127.0.0.1:3112"));
    assert!(is_loopback_host("127.0.0.5"));
    assert!(is_loopback_host("localhost"));
    assert!(is_loopback_host("localhost:8080"));
    assert!(is_loopback_host("[::1]"));
    assert!(is_loopback_host("[::1]:3112"));
    assert!(is_loopback_host("::1"));

    assert!(!is_loopback_host("0.0.0.0"));
    assert!(!is_loopback_host("10.0.0.5"));
    assert!(!is_loopback_host("attacker.example.com"));
    assert!(!is_loopback_host("evil.localhost.attacker.com"));
}
