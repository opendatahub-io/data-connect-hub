//! Handshake-level tests for the cluster-TLS-profile acceptor path.
//!
//! `build_tls_acceptor` implements the adherence contract for the Flight
//! listener: enforced minimum version, restricted TLS 1.2 cipher suites,
//! fail-fast on minima the provider cannot serve (below TLS 1.2), and
//! fail-fast on cipher lists with no provider-supported suite. These tests
//! drive real handshakes against `tls_incoming` with self-signed ECDSA
//! certificates so regressions in the acceptor wiring surface as handshake
//! failures, not just config errors.

use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::AsyncReadExt;
use tokio_stream::StreamExt;

use super::{build_tls_acceptor, tls_incoming};
use crate::utils::TlsConfig;

const SUITE_AES128: &str = "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256";
const SUITE_AES256: &str = "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384";

/// Self-signed ECDSA certificate and key in temp PEM files.
struct TestCert {
    cert_path: tempfile::TempPath,
    key_path: tempfile::TempPath,
    der: CertificateDer<'static>,
}

fn self_signed_cert() -> TestCert {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("generating self-signed certificate");
    let cert_pem = certified.cert.pem();
    let key_pem = certified.key_pair.serialize_pem();
    let der = CertificateDer::from_pem_slice(cert_pem.as_bytes()).expect("parsing certificate PEM");

    let mut cert_file = tempfile::NamedTempFile::new().expect("cert temp file");
    std::io::Write::write_all(&mut cert_file, cert_pem.as_bytes()).expect("writing cert PEM");
    let mut key_file = tempfile::NamedTempFile::new().expect("key temp file");
    std::io::Write::write_all(&mut key_file, key_pem.as_bytes()).expect("writing key PEM");

    TestCert {
        cert_path: cert_file.into_temp_path(),
        key_path: key_file.into_temp_path(),
        der,
    }
}

fn root_store(cert: &TestCert) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der.clone()).expect("adding trust anchor");
    roots
}

fn provider_suite(name: &str) -> rustls::SupportedCipherSuite {
    rustls::crypto::aws_lc_rs::default_provider()
        .cipher_suites
        .into_iter()
        .find(|suite| suite.suite().as_str() == Some(name))
        .unwrap_or_else(|| panic!("suite {name} not offered by the provider"))
}

/// Binds a loopback listener and serves TLS handshakes through
/// `tls_incoming`, the same adapter `start_server` uses.
async fn spawn_tls_listener(acceptor: tokio_rustls::TlsAcceptor) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding loopback listener");
    let addr = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        let incoming = tls_incoming(listener, acceptor);
        tokio::pin!(incoming);
        while let Some(stream) = incoming.next().await {
            if let Ok(mut stream) = stream {
                // Hold the connection open until the test client is done,
                // bounded so a leaked task cannot outlive the test run.
                tokio::spawn(async move {
                    let mut buf = [0u8; 64];
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut buf)).await;
                });
            }
        }
    });
    addr
}

/// Connects a client restricted to `versions` and optionally to `suites`,
/// mirroring the provider chain the server uses.
async fn connect(
    addr: std::net::SocketAddr,
    roots: &rustls::RootCertStore,
    versions: &[&'static rustls::SupportedProtocolVersion],
    suites: Option<Vec<rustls::SupportedCipherSuite>>,
) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    // Mirror the server's provider chain: restricting the offered suites
    // requires a derived CryptoProvider in this rustls line.
    let base = rustls::crypto::aws_lc_rs::default_provider();
    let provider = Arc::new(rustls::crypto::CryptoProvider {
        cipher_suites: suites.unwrap_or_else(|| base.cipher_suites.clone()),
        ..base
    });
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .expect("building client config")
        .with_root_certificates(roots.clone())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let name = ServerName::try_from("localhost").expect("server name");
    connector.connect(name, tcp).await
}

#[tokio::test]
async fn modern_minimum_serves_tls13_only() {
    let cert = self_signed_cert();
    let acceptor = build_tls_acceptor(
        cert.cert_path.to_str().unwrap(),
        cert.key_path.to_str().unwrap(),
        "VersionTLS13",
        None,
    )
    .await
    .expect("building acceptor for Modern profile");
    let addr = spawn_tls_listener(acceptor).await;
    let roots = root_store(&cert);

    // TLS 1.3 clients complete the handshake with h2 negotiated.
    let stream = connect(addr, &roots, &[&rustls::version::TLS13], None)
        .await
        .expect("TLS 1.3 handshake");
    let (_, conn) = stream.get_ref();
    assert_eq!(conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
    assert_eq!(conn.alpn_protocol(), Some(b"h2".as_slice()));
    drop(stream);

    // TLS 1.2-only clients are rejected by the enforced minimum.
    let err = connect(addr, &roots, &[&rustls::version::TLS12], None)
        .await
        .expect_err("TLS 1.2 client against a TLS 1.3 minimum");
    assert!(
        err.to_string().contains("alert") || err.kind() == std::io::ErrorKind::UnexpectedEof,
        "unexpected error for rejected TLS 1.2 client: {err}"
    );
}

#[tokio::test]
async fn restricted_cipher_suite_enforced_for_tls12() {
    let cert = self_signed_cert();
    let acceptor = build_tls_acceptor(
        cert.cert_path.to_str().unwrap(),
        cert.key_path.to_str().unwrap(),
        "VersionTLS12",
        Some(&[SUITE_AES128.to_string()]),
    )
    .await
    .expect("building acceptor with restricted suites");
    let addr = spawn_tls_listener(acceptor).await;
    let roots = root_store(&cert);

    // A client offering the requested suite negotiates it over TLS 1.2.
    let stream = connect(
        addr,
        &roots,
        &[&rustls::version::TLS12],
        Some(vec![provider_suite(SUITE_AES128)]),
    )
    .await
    .expect("handshake with the projected suite");
    let (_, conn) = stream.get_ref();
    assert_eq!(conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_2));
    assert_eq!(
        conn.negotiated_cipher_suite().map(|s| s.suite()),
        Some(provider_suite(SUITE_AES128).suite())
    );
    drop(stream);

    // A client offering only a suite outside the projected list is refused,
    // and the listener keeps serving after the failed handshake.
    connect(
        addr,
        &roots,
        &[&rustls::version::TLS12],
        Some(vec![provider_suite(SUITE_AES256)]),
    )
    .await
    .expect_err("handshake with a non-projected suite");

    connect(
        addr,
        &roots,
        &[&rustls::version::TLS12],
        Some(vec![provider_suite(SUITE_AES128)]),
    )
    .await
    .expect("listener still serving after refused handshake");
}

#[tokio::test]
async fn tls12_minimum_accepts_tls12_clients() {
    let cert = self_signed_cert();
    let acceptor = build_tls_acceptor(
        cert.cert_path.to_str().unwrap(),
        cert.key_path.to_str().unwrap(),
        "VersionTLS12",
        None,
    )
    .await
    .expect("building acceptor for a TLS 1.2 minimum");
    let addr = spawn_tls_listener(acceptor).await;

    // Unrestricted suite list: a TLS 1.2-only client with provider defaults
    // still negotiates TLS 1.2.
    let stream = connect(addr, &root_store(&cert), &[&rustls::version::TLS12], None)
        .await
        .expect("TLS 1.2 handshake under the projected minimum");
    let (_, conn) = stream.get_ref();
    assert_eq!(conn.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_2));
}

#[tokio::test]
async fn old_profile_minima_fail_fast() {
    // rustls cannot serve TLS 1.0/1.1: the Old profile's minimum must be a
    // startup error, never a silent normalization to TLS 1.2.
    let cert = self_signed_cert();
    for min_version in ["VersionTLS10", "VersionTLS11"] {
        let err = match build_tls_acceptor(
            cert.cert_path.to_str().unwrap(),
            cert.key_path.to_str().unwrap(),
            min_version,
            None,
        )
        .await
        {
            Ok(_) => panic!("acceptor built for an Old-profile minimum {min_version}"),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("cannot serve below TLS 1.2"),
            "unexpected error for {min_version}: {err}"
        );
    }
}

#[tokio::test]
async fn unknown_minimum_version_fails_fast() {
    // An unvalidated minimum must be a configuration error, never a panic
    // or a silent fallback to provider defaults.
    let cert = self_signed_cert();
    let err = match build_tls_acceptor(
        cert.cert_path.to_str().unwrap(),
        cert.key_path.to_str().unwrap(),
        "VersionTLS42",
        None,
    )
    .await
    {
        Ok(_) => panic!("acceptor built for an unknown minimum version"),
        Err(err) => err.to_string(),
    };
    assert!(
        err.contains("unsupported TLS minimum version"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn unusable_cipher_lists_fail_fast() {
    let cert = self_signed_cert();
    // TLS 1.3-only suite names are unusable for TLS <= 1.2 handshakes, and
    // CBC suites (Old profile) are not implemented by the provider at all.
    for ciphers in [
        vec!["TLS_AES_128_GCM_SHA256".to_string()],
        vec!["TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA".to_string()],
    ] {
        let err = match build_tls_acceptor(
            cert.cert_path.to_str().unwrap(),
            cert.key_path.to_str().unwrap(),
            "VersionTLS12",
            Some(&ciphers),
        )
        .await
        {
            Ok(_) => panic!("acceptor built with an unusable cipher list {ciphers:?}"),
            Err(err) => err.to_string(),
        };
        assert!(
            err.contains("no usable cipher suites"),
            "unexpected error for {ciphers:?}: {err}"
        );
    }
}

#[tokio::test]
async fn legacy_config_without_policy_keeps_identity_path() {
    // Standalone configs (no projected min_version) keep the legacy
    // identity-only tonic TLS path: no enforced acceptor is returned.
    // tonic's ServerTlsConfig resolves the provider from the process
    // default, which main.rs installs before configuring TLS.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert = self_signed_cert();
    let tls = TlsConfig {
        cert_file: Some(cert.cert_path.to_str().unwrap().to_string()),
        key_file: Some(cert.key_path.to_str().unwrap().to_string()),
        min_version: None,
        ciphers: None,
    };
    let (_, acceptor) = super::configure_tls(tonic::transport::Server::builder(), &tls)
        .await
        .expect("legacy TLS configuration");
    assert!(acceptor.is_none());
}

#[tokio::test]
async fn projected_policy_without_certificates_fails_closed() {
    // A projected policy with incomplete certificate config must not start
    // the listener in plaintext.
    let tls = TlsConfig {
        cert_file: None,
        key_file: None,
        min_version: Some("VersionTLS12".to_string()),
        ciphers: None,
    };
    let err = match super::configure_tls(tonic::transport::Server::builder(), &tls).await {
        Ok(_) => panic!("projected policy without certificates must not configure"),
        Err(err) => err.to_string(),
    };
    assert!(err.contains("cert_file"), "unexpected error: {err}");
}
