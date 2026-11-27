//! Shared FIPS policy and OpenSSL-backed Rustls provider initialization.

use anyhow::Result;

#[allow(unsafe_code)]
fn load_default_openssl_config() -> Result<()> {
    use ossl::bindings::{OPENSSL_INIT_LOAD_CONFIG, OPENSSL_init_crypto};
    use std::ptr::null;

    let options = OPENSSL_INIT_LOAD_CONFIG as u64;
    if unsafe { OPENSSL_init_crypto(options, null()) } != 1 {
        anyhow::bail!("failed to load the OpenSSL configuration");
    }

    Ok(())
}

/// Preference order for the approved curves. rustls speculates a key share
/// for the first group only, so this decides what every ClientHello carries:
/// P-256 is the one every peer supports and the cheapest to generate, and
/// leading with it avoids a HelloRetryRequest against servers that prefer it.
fn fips_kx_group_rank(group: rustls::NamedGroup) -> Option<u8> {
    match group {
        rustls::NamedGroup::secp256r1 => Some(0),
        rustls::NamedGroup::secp384r1 => Some(1),
        rustls::NamedGroup::secp521r1 => Some(2),
        _ => None,
    }
}

/// Drops key exchange groups that are not FIPS approved, X25519 among them,
/// and orders what remains by preference.
///
/// The selection is by group identity rather than by `group.fips()`, which
/// reports whether the OpenSSL context can supply the curve at all: off a
/// FIPS host that answers true for X25519 too.
fn retain_fips_kx_groups(provider: &mut rustls::crypto::CryptoProvider) {
    provider
        .kx_groups
        .retain(|group| fips_kx_group_rank(group.name()).is_some());
    provider
        .kx_groups
        .sort_by_key(|group| fips_kx_group_rank(group.name()).unwrap_or(u8::MAX));
}

/// Key exchange groups are restricted to the NIST curves first. `rustls`
/// computes `CryptoProvider::fips()` as an `all()` across every configured
/// component, and derives `ClientConfig::require_ems` from that value, so a
/// single component reporting non-FIPS would silently disable Extended Master
/// Secret — mandatory for the TLS 1.2 KDF under FIPS 140-3 IG section D.Q.
///
/// When `DCH_REQUIRE_FIPS` is set, a configuration that does not report FIPS
/// is treated as fatal rather than degrading quietly.
///
/// The status is returned rather than logged: this has to run before service
/// tracing initialization, which may open an OTLP exporter over TLS, so no
/// subscriber exists yet and anything logged here would be discarded.
pub struct CryptoStatus {
    context_fips: bool,
    provider_fips: bool,
    cipher_suites_fips: bool,
    kx_groups_fips: bool,
    signature_verification_fips: bool,
    secure_random_fips: bool,
    key_provider_fips: bool,
    config_fips: bool,
    kx_groups: Vec<rustls::NamedGroup>,
    cipher_suites: Vec<rustls::CipherSuite>,
    signature_schemes: Vec<rustls::SignatureScheme>,
}

impl CryptoStatus {
    pub fn log(&self) {
        tracing::info!(
            context_fips = self.context_fips,
            provider_fips = self.provider_fips,
            cipher_suites_fips = self.cipher_suites_fips,
            kx_groups_fips = self.kx_groups_fips,
            signature_verification_fips = self.signature_verification_fips,
            secure_random_fips = self.secure_random_fips,
            key_provider_fips = self.key_provider_fips,
            config_fips = self.config_fips,
            kx_groups = ?self.kx_groups,
            cipher_suites = ?self.cipher_suites,
            signature_schemes = ?self.signature_schemes,
            "installed OpenSSL-backed rustls crypto provider"
        );
    }
}

/// Installs the OpenSSL-backed Rustls provider as the process default, so TLS
/// cryptography is performed by system OpenSSL — and therefore, on a FIPS-mode
/// host, by the validated FIPS provider — rather than a statically linked Rust
/// implementation.
pub fn install_crypto_provider() -> Result<CryptoStatus> {
    use rustls::crypto::CryptoProvider;

    load_default_openssl_config()?;

    let mut provider = rustls_ossl::try_default_provider()
        .map_err(|e| anyhow::anyhow!("the OpenSSL crypto provider is unusable: {e}"))?;

    retain_fips_kx_groups(&mut provider);
    let kx_groups: Vec<_> = provider.kx_groups.iter().map(|group| group.name()).collect();
    // try_default_provider vouches for what the OpenSSL context supplied;
    // this guards what the policy applied here leaves behind. Every FIPS flag
    // below is an `all()`, which is true over an empty list, so a provider
    // filtered down to nothing would report flawless compliance while being
    // unable to complete a handshake.
    let cipher_suites: Vec<_> = provider.cipher_suites.iter().map(|suite| suite.suite()).collect();
    let signature_schemes: Vec<_> = provider
        .signature_verification_algorithms
        .mapping
        .iter()
        .map(|(scheme, _)| *scheme)
        .collect();

    if cipher_suites.is_empty() || kx_groups.is_empty() || signature_schemes.is_empty() {
        return Err(anyhow::anyhow!(
            "the configured FIPS policy left an incomplete algorithm set \
             (cipher suites: {}, kx groups: {}, signature algorithms: {}); refusing to start",
            cipher_suites.len(),
            kx_groups.len(),
            signature_schemes.len(),
        ));
    }

    let cipher_suites_fips = provider.cipher_suites.iter().all(|suite| suite.fips());
    let kx_groups_fips = provider.kx_groups.iter().all(|group| group.fips());
    let signature_verification_fips = provider.signature_verification_algorithms.fips();
    let secure_random_fips = provider.secure_random.fips();
    let key_provider_fips = provider.key_provider.fips();
    let provider_fips = provider.fips();
    let context_fips = rustls_ossl::osslctx().fips_is_enabled();

    CryptoProvider::install_default(provider)
        .map_err(|_| anyhow::anyhow!("a rustls CryptoProvider was already installed"))?;

    // Build a throwaway client configuration purely to read the config-level
    // FIPS status: it folds in `require_ems`, which the provider-level flag
    // does not cover.
    let probe = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let config_fips = probe.fips();

    if std::env::var_os("DCH_REQUIRE_FIPS").is_some() && !config_fips {
        return Err(anyhow::anyhow!(
            "DCH_REQUIRE_FIPS is set but the TLS configuration does not report FIPS \
             (provider_fips={provider_fips}, config_fips={config_fips}); refusing to start"
        ));
    }

    Ok(CryptoStatus {
        context_fips,
        provider_fips,
        cipher_suites_fips,
        kx_groups_fips,
        signature_verification_fips,
        secure_random_fips,
        key_provider_fips,
        config_fips,
        kx_groups,
        cipher_suites,
        signature_schemes,
    })
}
