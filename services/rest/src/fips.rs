//! Local FIPS policy on top of the rustls-ossl provider.
//!
//! Which suites to offer is a policy choice and belongs here. Everything that
//! once compensated for a defect in the provider -- the RNG, the TLS KDFs, the
//! AEAD and signature FIPS reporting, the uncompressed key share and the
//! empty-provider check -- moved upstream; see the `[patch.crates-io]`
//! section of the workspace Cargo.toml.

/// Preference order for the approved curves. rustls speculates a key share
/// for the first group only, so this decides what every ClientHello carries:
/// P-256 is the one every peer supports and the cheapest to generate, and
/// leading with it avoids a HelloRetryRequest against servers that prefer it.
/// The others stay available for the retry.
fn fips_kx_group_rank(group: rustls::NamedGroup) -> Option<u8> {
    match group {
        rustls::NamedGroup::secp256r1 => Some(0),
        rustls::NamedGroup::secp384r1 => Some(1),
        rustls::NamedGroup::secp521r1 => Some(2),
        _ => None,
    }
}

/// retain_fips_kx_groups drops the key exchange groups that are not FIPS
/// approved, X25519 among them, and orders what remains by preference.
///
/// The selection is by group identity rather than by `group.fips()`, which
/// reports whether the OpenSSL context can supply the curve at all: off a
/// FIPS host that answers true for X25519 too.
pub fn retain_fips_kx_groups(provider: &mut rustls::crypto::CryptoProvider) {
    provider
        .kx_groups
        .retain(|group| fips_kx_group_rank(group.name()).is_some());
    provider
        .kx_groups
        .sort_by_key(|group| fips_kx_group_rank(group.name()).unwrap_or(u8::MAX));
}

fn is_fips_cipher_suite(suite: rustls::CipherSuite) -> bool {
    matches!(
        suite,
        rustls::CipherSuite::TLS13_AES_128_GCM_SHA256
            | rustls::CipherSuite::TLS13_AES_256_GCM_SHA384
            | rustls::CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            | rustls::CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
            | rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            | rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
    )
}

/// retain_fips_cipher_suites drops the suites that are not FIPS approved,
/// ChaCha20-Poly1305 among them. `CryptoProvider::fips()` is an `all()` over
/// the configured suites, so leaving one non-approved suite in the list makes
/// the whole provider report non-FIPS.
///
/// The selection is by suite identity rather than by `suite.fips()`, which
/// reports the state of the OpenSSL context: off a FIPS host every suite
/// answers false there, and filtering on it would leave no suite at all.
pub fn retain_fips_cipher_suites(provider: &mut rustls::crypto::CryptoProvider) {
    provider
        .cipher_suites
        .retain(|suite| is_fips_cipher_suite(suite.suite()));
}
