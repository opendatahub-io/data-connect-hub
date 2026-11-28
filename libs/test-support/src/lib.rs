//! Test-only helpers shared by DCH crates. Depend on this crate only from
//! `[dev-dependencies]`.

use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A local HTTPS server returning `200 OK` with body `{}` to every request.
pub struct HttpsServer {
    /// Base URL, for example `https://127.0.0.1:43210/`.
    pub url: String,
    /// PEM bundle of two unrelated CAs. Only the second one issued the server
    /// certificate, so a client reaches the server only if it trusts every
    /// certificate in the bundle, not just the first.
    pub ca_bundle_pem: String,
}

/// Start an HTTPS server on `127.0.0.1` whose certificate is issued by the
/// second CA in [`HttpsServer::ca_bundle_pem`]. The server sends only its leaf
/// certificate, so the issuing CA must come from the client's trust store.
pub async fn spawn_https_server_issued_by_second_ca() -> HttpsServer {
    let (unrelated_ca, _) = new_ca("dch-test-unrelated-ca");
    let (issuer_ca, issuer_key) = new_ca("dch-test-issuer-ca");

    let server_key = KeyPair::generate().expect("generate server key");
    let mut params = CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
        .expect("server certificate params");
    params.distinguished_name.push(DnType::CommonName, "dch-test-server");
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = params
        .signed_by(&server_key, &issuer_ca, &issuer_key)
        .expect("sign server certificate");

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![server_cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
        )
        .expect("server TLS config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
    let addr = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A rejected handshake is the failure the tests look for, and
                // the client reports it, so the server just drops the connection.
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                if read_request(&mut tls).await.is_ok() {
                    // Write errors surface on the client side as a failed request.
                    let _ = tls
                        .write_all(
                            b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                        )
                        .await;
                    let _ = tls.shutdown().await;
                }
            });
        }
    });

    HttpsServer {
        url: format!("https://127.0.0.1:{}/", addr.port()),
        ca_bundle_pem: format!("{}{}", unrelated_ca.pem(), issuer_ca.pem()),
    }
}

fn new_ca(common_name: &str) -> (rcgen::Certificate, KeyPair) {
    let key = KeyPair::generate().expect("generate CA key");
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    params.distinguished_name.push(DnType::CommonName, common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let cert = params.self_signed(&key).expect("self-sign CA");
    (cert, key)
}

/// Read one HTTP/1.1 request, including a `content-length` body, so the
/// response is not cut off by a reset from unread request bytes.
async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
    let body_len = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut remaining = body_len.saturating_sub(buf.len() - header_end);
    while remaining > 0 {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        remaining = remaining.saturating_sub(n);
    }
    Ok(())
}
