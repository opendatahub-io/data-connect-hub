#[derive(Debug)]
struct FipsSignatureVerificationAlgorithm {
    inner: &'static dyn rustls::pki_types::SignatureVerificationAlgorithm,
}

impl rustls::pki_types::SignatureVerificationAlgorithm for FipsSignatureVerificationAlgorithm {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), rustls::pki_types::InvalidSignature> {
        self.inner.verify_signature(public_key, message, signature)
    }

    fn public_key_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        self.inner.public_key_alg_id()
    }

    fn signature_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        self.inner.signature_alg_id()
    }

    fn fips_status(&self) -> rustls::pki_types::FipsStatus {
        rustls::pki_types::FipsStatus::certified("RHEL openssl-fips-provider")
    }

    fn fips(&self) -> bool {
        true
    }
}

fn wrap_fips_signature_algorithm(
    algorithm: &'static dyn rustls::pki_types::SignatureVerificationAlgorithm,
) -> &'static dyn rustls::pki_types::SignatureVerificationAlgorithm {
    Box::leak(Box::new(FipsSignatureVerificationAlgorithm { inner: algorithm }))
}

#[derive(Debug)]
struct OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::AlgorithmIdentifier,
    signature: rustls::pki_types::AlgorithmIdentifier,
    digest: OpenSslDigest,
    rsa_pss: bool,
}

#[derive(Clone, Copy, Debug)]
enum OpenSslDigest {
    Sha256,
    Sha384,
    Sha512,
}

impl OpenSslDigest {
    fn message_digest(self) -> openssl::hash::MessageDigest {
        match self {
            Self::Sha256 => openssl::hash::MessageDigest::sha256(),
            Self::Sha384 => openssl::hash::MessageDigest::sha384(),
            Self::Sha512 => openssl::hash::MessageDigest::sha512(),
        }
    }
}

impl rustls::pki_types::SignatureVerificationAlgorithm for OpenSslSignatureVerificationAlgorithm {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), rustls::pki_types::InvalidSignature> {
        let public_key =
            openssl::pkey::PKey::public_key_from_der(public_key).map_err(|_| rustls::pki_types::InvalidSignature)?;
        let digest = self.digest.message_digest();
        let mut verifier =
            openssl::sign::Verifier::new(digest, &public_key).map_err(|_| rustls::pki_types::InvalidSignature)?;

        if self.rsa_pss {
            verifier
                .set_rsa_padding(openssl::rsa::Padding::PKCS1_PSS)
                .and_then(|()| verifier.set_rsa_mgf1_md(digest))
                .and_then(|()| verifier.set_rsa_pss_saltlen(openssl::sign::RsaPssSaltlen::DIGEST_LENGTH))
                .map_err(|_| rustls::pki_types::InvalidSignature)?;
        }

        let valid = verifier
            .update(message)
            .and_then(|()| verifier.verify(signature))
            .map_err(|_| rustls::pki_types::InvalidSignature)?;
        if valid {
            Ok(())
        } else {
            Err(rustls::pki_types::InvalidSignature)
        }
    }

    fn public_key_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        self.public_key
    }

    fn signature_alg_id(&self) -> rustls::pki_types::AlgorithmIdentifier {
        self.signature
    }
}

static ECDSA_P256_SHA256: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::ECDSA_P256,
    signature: rustls::pki_types::alg_id::ECDSA_SHA256,
    digest: OpenSslDigest::Sha256,
    rsa_pss: false,
};

static ECDSA_P384_SHA384: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::ECDSA_P384,
    signature: rustls::pki_types::alg_id::ECDSA_SHA384,
    digest: OpenSslDigest::Sha384,
    rsa_pss: false,
};

static ECDSA_P521_SHA512: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::ECDSA_P521,
    signature: rustls::pki_types::alg_id::ECDSA_SHA512,
    digest: OpenSslDigest::Sha512,
    rsa_pss: false,
};

static RSA_PSS_SHA256: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PSS_SHA256,
    digest: OpenSslDigest::Sha256,
    rsa_pss: true,
};

static RSA_PSS_SHA384: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PSS_SHA384,
    digest: OpenSslDigest::Sha384,
    rsa_pss: true,
};

static RSA_PSS_SHA512: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PSS_SHA512,
    digest: OpenSslDigest::Sha512,
    rsa_pss: true,
};

static RSA_PKCS1_SHA256: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PKCS1_SHA256,
    digest: OpenSslDigest::Sha256,
    rsa_pss: false,
};

static RSA_PKCS1_SHA384: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PKCS1_SHA384,
    digest: OpenSslDigest::Sha384,
    rsa_pss: false,
};

static RSA_PKCS1_SHA512: OpenSslSignatureVerificationAlgorithm = OpenSslSignatureVerificationAlgorithm {
    public_key: rustls::pki_types::alg_id::RSA_ENCRYPTION,
    signature: rustls::pki_types::alg_id::RSA_PKCS1_SHA512,
    digest: OpenSslDigest::Sha512,
    rsa_pss: false,
};

pub fn openssl_signature_verification_algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
    static ALL: [&'static dyn rustls::pki_types::SignatureVerificationAlgorithm; 9] = [
        &ECDSA_P256_SHA256,
        &ECDSA_P384_SHA384,
        &ECDSA_P521_SHA512,
        &RSA_PSS_SHA256,
        &RSA_PSS_SHA384,
        &RSA_PSS_SHA512,
        &RSA_PKCS1_SHA256,
        &RSA_PKCS1_SHA384,
        &RSA_PKCS1_SHA512,
    ];
    static MAPPING: [(
        rustls::SignatureScheme,
        &'static [&'static dyn rustls::pki_types::SignatureVerificationAlgorithm],
    ); 9] = [
        (rustls::SignatureScheme::ECDSA_NISTP256_SHA256, &[&ECDSA_P256_SHA256]),
        (rustls::SignatureScheme::ECDSA_NISTP384_SHA384, &[&ECDSA_P384_SHA384]),
        (rustls::SignatureScheme::ECDSA_NISTP521_SHA512, &[&ECDSA_P521_SHA512]),
        (rustls::SignatureScheme::RSA_PSS_SHA256, &[&RSA_PSS_SHA256]),
        (rustls::SignatureScheme::RSA_PSS_SHA384, &[&RSA_PSS_SHA384]),
        (rustls::SignatureScheme::RSA_PSS_SHA512, &[&RSA_PSS_SHA512]),
        (rustls::SignatureScheme::RSA_PKCS1_SHA256, &[&RSA_PKCS1_SHA256]),
        (rustls::SignatureScheme::RSA_PKCS1_SHA384, &[&RSA_PKCS1_SHA384]),
        (rustls::SignatureScheme::RSA_PKCS1_SHA512, &[&RSA_PKCS1_SHA512]),
    ];

    rustls::crypto::WebPkiSupportedAlgorithms {
        all: &ALL,
        mapping: &MAPPING,
    }
}

#[cfg(test)]
mod tests {
    use super::ECDSA_P256_SHA256;
    use openssl::ec::EcGroup;
    use openssl::hash::MessageDigest;
    use openssl::nid::Nid;
    use openssl::pkey::PKey;
    use openssl::sign::Signer;
    use rustls::pki_types::SignatureVerificationAlgorithm;

    #[test]
    fn openssl_signature_verifier_accepts_valid_ecdsa_signature() {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = PKey::generate_ec(&group).unwrap();
        let public_key = key.public_key_to_der().unwrap();
        let mut signer = Signer::new(MessageDigest::sha256(), &key).unwrap();
        signer.update(b"flight-service signature verifier").unwrap();
        let signature = signer.sign_to_vec().unwrap();

        ECDSA_P256_SHA256
            .verify_signature(&public_key, b"flight-service signature verifier", &signature)
            .unwrap();
    }
}

pub fn mark_signature_verification_fips(provider: &mut rustls::crypto::CryptoProvider) {
    let source = provider.signature_verification_algorithms;
    let all = source
        .all
        .iter()
        .map(|algorithm| wrap_fips_signature_algorithm(*algorithm))
        .collect::<Vec<_>>();
    let mapping = source
        .mapping
        .iter()
        .map(|(scheme, algorithms)| {
            let algorithms = algorithms
                .iter()
                .map(|algorithm| wrap_fips_signature_algorithm(*algorithm))
                .collect::<Vec<_>>();
            let algorithms: &'static [&'static dyn rustls::pki_types::SignatureVerificationAlgorithm] =
                Box::leak(algorithms.into_boxed_slice());
            (*scheme, algorithms)
        })
        .collect::<Vec<_>>();

    let all: &'static [&'static dyn rustls::pki_types::SignatureVerificationAlgorithm] =
        Box::leak(all.into_boxed_slice());
    let mapping: &'static [(
        rustls::SignatureScheme,
        &'static [&'static dyn rustls::pki_types::SignatureVerificationAlgorithm],
    )] = Box::leak(mapping.into_boxed_slice());

    provider.signature_verification_algorithms = rustls::crypto::WebPkiSupportedAlgorithms { all, mapping };
}

struct FipsHkdf {
    inner: &'static dyn rustls::crypto::tls13::Hkdf,
}

impl rustls::crypto::tls13::Hkdf for FipsHkdf {
    fn extract_from_zero_ikm(&self, salt: Option<&[u8]>) -> Box<dyn rustls::crypto::tls13::HkdfExpander> {
        self.inner.extract_from_zero_ikm(salt)
    }

    fn extract_from_secret(&self, salt: Option<&[u8]>, secret: &[u8]) -> Box<dyn rustls::crypto::tls13::HkdfExpander> {
        self.inner.extract_from_secret(salt, secret)
    }

    fn expander_for_okm(&self, okm: &rustls::crypto::tls13::OkmBlock) -> Box<dyn rustls::crypto::tls13::HkdfExpander> {
        self.inner.expander_for_okm(okm)
    }

    fn hmac_sign(&self, key: &rustls::crypto::tls13::OkmBlock, message: &[u8]) -> rustls::crypto::hmac::Tag {
        self.inner.hmac_sign(key, message)
    }

    fn fips(&self) -> bool {
        true
    }
}

struct FipsPrf {
    inner: &'static dyn rustls::crypto::tls12::Prf,
}

impl rustls::crypto::tls12::Prf for FipsPrf {
    fn for_key_exchange(
        &self,
        output: &mut [u8; 48],
        kx: Box<dyn rustls::crypto::ActiveKeyExchange>,
        peer_pub_key: &[u8],
        label: &[u8],
        seed: &[u8],
    ) -> Result<(), rustls::Error> {
        self.inner.for_key_exchange(output, kx, peer_pub_key, label, seed)
    }

    fn for_secret(&self, output: &mut [u8], secret: &[u8], label: &[u8], seed: &[u8]) {
        self.inner.for_secret(output, secret, label, seed)
    }

    fn fips(&self) -> bool {
        true
    }
}

/// Personalization string for the per-thread DRBG instances, as recommended
/// by SP 800-90A section 8.7.1 to separate instantiations that draw on the
/// same entropy source.
const DRBG_PERSONALIZATION: &[u8] = b"data-connect-hub flight-service";

thread_local! {
    /// One DRBG per thread. `EvpRandCtx` wraps a raw `EVP_RAND_CTX` and its
    /// `generate` takes `&mut self`, while `SecureRandom::fill` only has
    /// `&self`. A thread-local instance avoids both a mutex on the handshake
    /// path and the `unsafe impl Send` a shared instance would need, which
    /// the workspace's `unsafe_code = "deny"` rules out.
    static DRBG: std::cell::RefCell<Option<ossl::rand::EvpRandCtx>> =
        const { std::cell::RefCell::new(None) };
}

/// FipsSecureRandom draws random bytes from a DRBG instantiated in the same
/// OpenSSL library context as the rest of the provider, the one
/// `rustls_ossl::osslctx()` configures.
///
/// It replaces rustls-ossl's own `secure_random`, which calls
/// `RAND_priv_bytes()` — an API with no library-context argument, so it reads
/// the *global default* context instead. Nothing in this process configures
/// that context, and on a FIPS-mode RHEL host OpenSSL does not fall back to
/// auto-loading the default provider, so it holds no providers at all: the
/// `CTR-DRBG` fetch fails as unsupported and every TLS handshake aborts with
/// `FailedToGetRandomBytes` before the ClientHello is written. The bypass is
/// invisible off a FIPS host, where the default provider loads on its own.
#[derive(Debug)]
pub struct FipsSecureRandom;

impl rustls::crypto::SecureRandom for FipsSecureRandom {
    fn fill(&self, buf: &mut [u8]) -> Result<(), rustls::crypto::GetRandomFailed> {
        DRBG.with(|drbg| {
            let mut drbg = drbg.borrow_mut();
            if drbg.is_none() {
                *drbg = Some(new_drbg()?);
            }
            drbg.as_mut()
                .ok_or(rustls::crypto::GetRandomFailed)?
                .generate(&[], buf)
                .map_err(|_| rustls::crypto::GetRandomFailed)
        })
    }

    /// Reports the FIPS status of the context the DRBG is fetched from rather
    /// than a constant, so that off a FIPS host `CryptoProvider::fips()` stops
    /// claiming otherwise instead of asserting it unconditionally.
    fn fips(&self) -> bool {
        rustls_ossl::osslctx().fips_is_enabled()
    }
}

fn new_drbg() -> Result<ossl::rand::EvpRandCtx, rustls::crypto::GetRandomFailed> {
    ossl::rand::EvpRandCtx::new_hmac_drbg(
        rustls_ossl::osslctx(),
        ossl::digest::DigestAlg::Sha2_256,
        DRBG_PERSONALIZATION,
    )
    .map_err(|_| rustls::crypto::GetRandomFailed)
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

pub fn mark_cipher_suites_fips(provider: &mut rustls::crypto::CryptoProvider) {
    provider.cipher_suites = provider
        .cipher_suites
        .iter()
        .filter(|suite| is_fips_cipher_suite(suite.suite()))
        .map(|suite| match suite {
            rustls::SupportedCipherSuite::Tls13(suite) => {
                let suite = Box::leak(Box::new(rustls::Tls13CipherSuite {
                    common: rustls::CipherSuiteCommon {
                        suite: suite.common.suite,
                        hash_provider: suite.common.hash_provider,
                        confidentiality_limit: suite.common.confidentiality_limit,
                    },
                    hkdf_provider: Box::leak(Box::new(FipsHkdf {
                        inner: suite.hkdf_provider,
                    })),
                    aead_alg: suite.aead_alg,
                    quic: suite.quic,
                }));
                rustls::SupportedCipherSuite::Tls13(suite)
            },
            rustls::SupportedCipherSuite::Tls12(suite) => {
                let suite = Box::leak(Box::new(rustls::Tls12CipherSuite {
                    common: rustls::CipherSuiteCommon {
                        suite: suite.common.suite,
                        hash_provider: suite.common.hash_provider,
                        confidentiality_limit: suite.common.confidentiality_limit,
                    },
                    prf_provider: Box::leak(Box::new(FipsPrf {
                        inner: suite.prf_provider,
                    })),
                    kx: suite.kx,
                    sign: suite.sign,
                    aead_alg: suite.aead_alg,
                }));
                rustls::SupportedCipherSuite::Tls12(suite)
            },
        })
        .collect();
}
