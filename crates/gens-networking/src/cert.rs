//! Self-signed certificates for loopback QUIC (Phase 6).
//!
//! The transport's CI posture is **loopback-first**: endpoints talk to
//! 127.0.0.1 with an in-memory self-signed certificate generated at
//! bind time. Nothing is written to disk, nothing trusts the network,
//! and no CA ceremony exists — the client pins the exact certificate
//! the server presents (one `RootCertStore` entry).
//!
//! Provider parity matters here: quinn is pinned with the `rustls-ring`
//! crypto provider and rcgen with the `ring` feature, so both sides of
//! the handshake use the same backend (no aws-lc-rs build burden on
//! Windows CI — the decision is recorded in docs/WORKLOG.md P6-0).

use quinn::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

/// The DNS name the client uses in the TLS handshake for loopback.
pub const LOOPBACK_SERVER_NAME: &str = "localhost";

/// A generated loopback identity: one self-signed certificate and its
/// signing key, both as DER for direct quinn consumption.
#[derive(Debug, Clone)]
pub struct LoopbackCert {
    cert_der: CertificateDer<'static>,
    key_der: Vec<u8>,
}

impl LoopbackCert {
    /// Generates a fresh self-signed certificate for `localhost`.
    ///
    /// Fresh randomness per call is fine: identity is per-endpoint,
    /// and the client learns the certificate from the operator (the
    /// engine passes the DER across explicitly, never via a network
    /// fetch — no TOFU, no trust on first use).
    pub fn generate() -> Result<LoopbackCert, rcgen::Error> {
        let certified = rcgen::generate_simple_self_signed(vec![LOOPBACK_SERVER_NAME.to_string()])?;
        Ok(LoopbackCert {
            cert_der: certified.cert.der().clone(),
            key_der: certified.signing_key.serialize_der(),
        })
    }

    /// The certificate (to hand to clients that should trust us).
    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.cert_der
    }

    /// Builds the quinn server config presenting this certificate.
    pub fn server_config(&self) -> Result<quinn::ServerConfig, quinn::rustls::Error> {
        let key = PrivatePkcs8KeyDer::from(self.key_der.clone());
        quinn::ServerConfig::with_single_cert(vec![self.cert_der.clone()], key.into())
    }

    /// Builds the quinn client config trusting exactly this certificate
    /// (and nothing else — no system roots, no network trust).
    pub fn client_config(&self) -> Result<quinn::ClientConfig, String> {
        let mut roots = quinn::rustls::RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates([self.cert_der.clone()]);
        if added != 1 || ignored != 0 {
            return Err(format!(
                "loopback root store rejected our own certificate (added {added}, ignored {ignored})"
            ));
        }
        quinn::ClientConfig::with_root_certificates(std::sync::Arc::new(roots))
            .map_err(|e| format!("client config build failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_parsable_certificate_and_key() {
        let cert = LoopbackCert::generate().expect("generate");
        assert!(!cert.cert_der.as_ref().is_empty(), "cert DER present");
        assert!(!cert.key_der.is_empty(), "key DER present");
        // DER SEQUENCE tag: a well-formed certificate starts with 0x30.
        assert_eq!(cert.cert_der.as_ref()[0], 0x30, "DER SEQUENCE tag");
        // PKCS#8 key blobs also start with a SEQUENCE.
        assert_eq!(cert.key_der[0], 0x30, "PKCS#8 SEQUENCE tag");
    }

    #[test]
    fn configs_build_from_the_same_certificate() {
        let cert = LoopbackCert::generate().expect("generate");
        cert.server_config().expect("server config");
        cert.client_config().expect("client config");
    }

    #[test]
    fn two_generations_are_independent() {
        let a = LoopbackCert::generate().expect("a");
        let b = LoopbackCert::generate().expect("b");
        assert_ne!(
            a.cert_der.as_ref(),
            b.cert_der.as_ref(),
            "fresh randomness per generation"
        );
    }
}
