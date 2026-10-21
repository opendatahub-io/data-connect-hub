pub mod flight;
pub mod utils;

#[cfg(test)]
mod tls_tests;

use anyhow::Result;
use arrow_flight::flight_service_server::FlightServiceServer;
use clap::Parser;
use config::{Config, File};
use flight::DataIngestionService;
use flight::auth::AuthLayer;
use flight::metrics::{install_prometheus_recorder, spawn_metrics_server};
use flight::trace::TraceLayer;
use kube_utils::KubeAuthClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use utils::ServerConfig;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct CommandLineArgs {
    /// Enable JSON logs
    #[arg(short, long, default_value = "false")]
    pub json_logs: bool,

    /// Config file for this server
    #[arg(short, long, default_value = "config/config.toml")]
    pub config: String,

    /// Optional additional config file (e.g. a mounted Secret) merged on top
    /// of `config`; missing values here fall back to `config`.
    #[arg(long, default_value = "/secrets/secret-config.toml")]
    pub secret_config: String,
}

pub fn load_config(config_file: String, secret_config_file: String) -> Result<ServerConfig> {
    let config = Config::builder()
        .add_source(File::with_name(config_file.as_str()))
        .add_source(File::with_name(secret_config_file.as_str()).required(false))
        .build()?;

    let config: ServerConfig = config.try_deserialize()?;
    Ok(config)
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => println!("\nReceived Ctrl+C, shutting down gracefully..."),
        _ = terminate => println!("\nReceived SIGTERM, shutting down gracefully..."),
    }
}

pub async fn configure_tls(
    mut builder: tonic::transport::Server,
    tls: &utils::TlsConfig,
) -> Result<(tonic::transport::Server, Option<tokio_rustls::TlsAcceptor>)> {
    tls.validate().map_err(|e| anyhow::anyhow!(e))?;
    if tls.min_version.is_some() && (tls.cert_file.is_none() || tls.key_file.is_none()) {
        // A projected cluster policy means the listener is managed: serving
        // plaintext because the certificate config is incomplete would fail
        // open, so refuse to start instead.
        return Err(anyhow::anyhow!(
            "cluster TLS policy projected (min_version set) but tls.cert_file/tls.key_file are not both configured"
        ));
    }
    let (Some(cert_file), Some(key_file)) = (&tls.cert_file, &tls.key_file) else {
        tracing::warn!("TLS is DISABLED — gRPC traffic is unencrypted");
        return Ok((builder, None));
    };
    match tls.min_version.as_deref() {
        None => {
            // No projected policy (standalone config): keep the identity-only
            // tonic TLS path with provider defaults.
            let cert = tokio::fs::read(cert_file).await?;
            let key = tokio::fs::read(key_file).await?;
            let identity = tonic::transport::Identity::from_pem(cert, key);
            let tls_config = tonic::transport::ServerTlsConfig::new().identity(identity);
            builder = builder.tls_config(tls_config)?;
            tracing::info!("TLS enabled (cert: {}, key: {})", cert_file, key_file);
            Ok((builder, None))
        },
        Some(min_version) => {
            let acceptor = build_tls_acceptor(cert_file, key_file, min_version, tls.ciphers.as_deref()).await?;
            tracing::info!("TLS enabled with enforced minimum {min_version} (cert: {cert_file}, key: {key_file})");
            Ok((builder, Some(acceptor)))
        },
    }
}

/// Builds the enforced-policy TLS acceptor from the projected cluster profile.
/// Fails fast on minima the provider cannot serve (below TLS 1.2) and on
/// cipher lists with no provider-supported suite, instead of starting with
/// defaults.
async fn build_tls_acceptor(
    cert_file: &str,
    key_file: &str,
    min_version: &str,
    ciphers: Option<&[String]>,
) -> Result<tokio_rustls::TlsAcceptor> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use std::sync::Arc;

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_file)
        .map_err(|e| anyhow::anyhow!("reading certificate file {cert_file}: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("parsing certificate file {cert_file}: {e}"))?;
    let key =
        PrivateKeyDer::from_pem_file(key_file).map_err(|e| anyhow::anyhow!("reading key file {key_file}: {e}"))?;

    let base_provider = rustls::crypto::aws_lc_rs::default_provider();
    let versions: &[&'static rustls::SupportedProtocolVersion] = match min_version {
        "VersionTLS13" => &[&rustls::version::TLS13],
        "VersionTLS12" => &[&rustls::version::TLS12, &rustls::version::TLS13],
        other => {
            return Err(anyhow::anyhow!(
                "unsupported TLS minimum version {other:?}: the Flight listener cannot serve below TLS 1.2"
            ));
        },
    };

    let mut suites: Vec<rustls::SupportedCipherSuite> = Vec::new();
    if min_version != "VersionTLS13" {
        if let Some(requested) = ciphers {
            let usable: Vec<rustls::SupportedCipherSuite> = base_provider
                .cipher_suites
                .iter()
                .filter(|suite| {
                    matches!(suite, rustls::SupportedCipherSuite::Tls12(_))
                        && requested
                            .iter()
                            .any(|name| suite.suite().as_str() == Some(name.as_str()))
                })
                .cloned()
                .collect();
            if usable.is_empty() {
                let supported: Vec<&str> = base_provider
                    .cipher_suites
                    .iter()
                    .filter_map(|s| s.suite().as_str())
                    .collect();
                return Err(anyhow::anyhow!(
                    "no usable cipher suites for TLS <= 1.2 in {requested:?}; provider supports {supported:?}"
                ));
            }
            suites = usable;
        } else {
            // No projected list: keep every TLS 1.2 suite the provider offers
            // instead of leaving TLS 1.2 handshakes without any suite.
            suites = base_provider
                .cipher_suites
                .iter()
                .filter(|suite| matches!(suite, rustls::SupportedCipherSuite::Tls12(_)))
                .cloned()
                .collect();
        }
    }
    // TLS 1.3 suites stay runtime-managed: always offer all of them for 1.3
    // handshakes so a TLS 1.2 minimum does not disable TLS 1.3 negotiation.
    for suite in &base_provider.cipher_suites {
        if matches!(suite, rustls::SupportedCipherSuite::Tls13(_)) && !suites.iter().any(|s| s.suite() == suite.suite())
        {
            suites.push(*suite);
        }
    }
    // The selected suites are enforced through a derived provider: this
    // rustls line takes cipher suites from the CryptoProvider only.
    let provider = Arc::new(rustls::crypto::CryptoProvider {
        cipher_suites: suites,
        ..base_provider
    });

    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"h2".to_vec()];
    Ok(tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config)))
}

/// Wraps accepted TCP connections in TLS handshakes, running up to 128
/// handshakes concurrently so a stalled client cannot block accepts.
fn tls_incoming(
    listener: tokio::net::TcpListener,
    acceptor: tokio_rustls::TlsAcceptor,
) -> impl futures::Stream<Item = std::io::Result<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>> {
    use tokio::sync::{Semaphore, mpsc};
    use tokio_stream::wrappers::ReceiverStream;

    let (tx, rx) = mpsc::channel(128);
    let permits = Arc::new(Semaphore::new(1024));
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = tx.closed() => break,
                accepted = listener.accept() => accepted,
            };
            let (stream, _) = match accepted {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!("TCP accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                },
            };
            let permit = match permits.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    tracing::debug!("dropping TLS connection: handshake limit reached");
                    drop(stream);
                    continue;
                },
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let _permit = permit;
                match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await {
                    Ok(Ok(stream)) => {
                        let _ = tx.send(Ok(stream)).await;
                    },
                    Ok(Err(error)) => tracing::debug!("TLS handshake failed: {error}"),
                    Err(_) => tracing::debug!("TLS handshake timed out"),
                }
            });
        }
    });
    ReceiverStream::new(rx)
}

pub fn configure_metrics(config: &ServerConfig) -> Result<()> {
    if config.metrics.enabled {
        tracing::info!(
            "Prometheus metrics enabled on {}:{}",
            config.metrics.address,
            config.metrics.port
        );
        install_prometheus_recorder()?;
        spawn_metrics_server(config.metrics.address.clone(), config.metrics.port);
    } else {
        tracing::info!("Prometheus metrics disabled");
    }
    Ok(())
}

pub async fn start_server(
    builder: tonic::transport::Server,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
    auth: &utils::AuthConfig,
    data_service: DataIngestionService,
    addr: std::net::SocketAddr,
) -> Result<()> {
    let service = FlightServiceServer::new(data_service);
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<FlightServiceServer<DataIngestionService>>()
        .await;

    // Either keeps both branches on one router type so the TLS transport
    // choice below is a single serve path.
    let auth_layer = if auth.enabled {
        tracing::info!(
            "Auth enabled (cache TTL: {}s, token_review_audiences: {:?})",
            auth.cache_ttl_secs,
            auth.token_review_audiences
        );
        let kube_auth = KubeAuthClient::try_default(
            Duration::from_secs(auth.cache_ttl_secs),
            auth.token_review_audiences.clone(),
        )
        .await?;
        tower::util::Either::Left(AuthLayer::new(
            Arc::new(kube_auth),
            auth.discovery_service_account.clone(),
        ))
    } else {
        tracing::warn!("Auth is DISABLED — all requests are unauthenticated");
        tower::util::Either::Right(tower::layer::util::Identity::new())
    };

    let router = builder
        .layer(TraceLayer::new())
        .layer(auth_layer)
        .add_service(health_service)
        .add_service(service);

    match tls_acceptor {
        None => {
            router.serve_with_shutdown(addr, shutdown_signal()).await?;
        },
        Some(acceptor) => {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| anyhow::anyhow!("binding the Flight TLS listener on {addr}: {e}"))?;
            router
                .serve_with_incoming_shutdown(tls_incoming(listener, acceptor), shutdown_signal())
                .await?;
        },
    }
    Ok(())
}
