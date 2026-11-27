//! Local FIPS policy on top of the rustls-ossl provider.
//!
//! Which key exchange groups to offer is a policy choice and belongs here.
//! Everything that once compensated for a defect in the provider -- the RNG,
//! the TLS KDFs, the AEAD and signature FIPS reporting, the uncompressed key
//! share and the empty-provider check -- moved upstream; see the
//! `[patch.crates-io]` section of the workspace Cargo.toml.

use anyhow::Result;

#[allow(unsafe_code)]
pub fn load_default_openssl_config() -> Result<()> {
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
