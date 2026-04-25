// TLS configuration for muxtop-server.

use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use rcgen::{
    CertificateParams, DistinguishedName, DnType, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
    SanType,
};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{ServerConfig, version};

/// Validity window for auto-generated self-signed certificates (per ADR /
/// MED-S6: 90 days, with `not_before = now - 1h` for clock skew).
const CERT_VALIDITY_DAYS: i64 = 90;

/// TLS configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("no private key found in key file")]
    NoPrivateKey,

    #[error("no certificates found in cert file")]
    NoCertificates,

    #[error("TLS configuration error: {0}")]
    Rustls(#[from] tokio_rustls::rustls::Error),

    #[error("certificate generation error: {0}")]
    CertGen(#[from] rcgen::Error),
}

/// Build a `TlsAcceptor` from PEM-encoded certificate and key files.
///
/// **Pinned to TLS 1.3 only** (per LOW-S1 / ADR follow-up): TLS 1.2 client
/// hellos will be rejected at the handshake layer.
pub fn acceptor_from_pem(cert_path: &Path, key_path: &Path) -> Result<TlsAcceptor, TlsError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|_| TlsError::NoCertificates)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::NoCertificates)?;

    if certs.is_empty() {
        return Err(TlsError::NoCertificates);
    }

    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|_| TlsError::NoPrivateKey)?;

    // Pin to TLS 1.3 only — see SECURITY_REPORT LOW-S1.
    let config = ServerConfig::builder_with_protocol_versions(&[&version::TLS13])
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Generate a self-signed certificate and private key.
///
/// Per MED-S6, the certificate is built explicitly via `CertificateParams`:
/// - **SAN**: `iPAddress` if `hostname` parses as an IP literal, else
///   `dNSName`.
/// - **CN**: same string (so it shows up in `openssl x509 -subject`).
/// - **Algorithm**: pinned to ECDSA P-256 / SHA-256.
/// - **Validity**: 90 days, with `not_before = now - 1h` to absorb clock
///   skew between server and client.
///
/// Returns `(cert_pem, key_pem)` as PEM-encoded strings.
pub fn generate_self_signed(hostname: &str) -> Result<(String, String), TlsError> {
    // Parse the hostname: if it's an IP literal, emit an iPAddress SAN; if
    // not, fall back to a DNS-name SAN.
    let san = match IpAddr::from_str(hostname) {
        Ok(ip) => SanType::IpAddress(ip),
        Err(_) => SanType::DnsName(hostname.try_into()?),
    };

    let mut params = CertificateParams::default();
    params.subject_alt_names = vec![san];

    // CN = the bind IP literal (or DNS name) so the subject is meaningful in
    // tooling output.
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname);
    params.distinguished_name = dn;

    // Standard server cert key usages.
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];

    // 90-day validity, 1 hour of slack on `not_before` for clock skew.
    let now = OffsetDateTime::now_utc();
    params.not_before = now - TimeDuration::hours(1);
    params.not_after = now + TimeDuration::days(CERT_VALIDITY_DAYS);

    // Pin algorithm to ECDSA P-256 / SHA-256.
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let cert = params.self_signed(&key_pair)?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    Ok((cert_pem, key_pem))
}

/// Compute the SHA-256 fingerprint of a DER-encoded certificate.
pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    use std::fmt::Write;

    let digest = ring::digest::digest(&ring::digest::SHA256, cert_der);
    let bytes = digest.as_ref();

    let mut out = String::with_capacity(95);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        write!(out, "{byte:02X}").unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use x509_parser::prelude::*;

    #[test]
    fn test_generate_self_signed_cert() {
        let (cert_pem, key_pem) = generate_self_signed("localhost").unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_cert_fingerprint_is_hex() {
        let (cert_pem, _) = generate_self_signed("localhost").unwrap();
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert!(!certs.is_empty());
        let fp = cert_fingerprint(certs[0].as_ref());
        assert!(fp.contains(':'));
        assert_eq!(fp.len(), 95);
    }

    #[test]
    fn test_acceptor_from_pem_valid() {
        let (cert_pem, key_pem) = generate_self_signed("localhost").unwrap();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();

        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();

        let acceptor = acceptor_from_pem(cert_file.path(), key_file.path());
        assert!(acceptor.is_ok());
    }

    #[test]
    fn test_acceptor_from_pem_missing_file() {
        let result = acceptor_from_pem(
            Path::new("/nonexistent/cert.pem"),
            Path::new("/nonexistent/key.pem"),
        );
        assert!(result.is_err());
    }

    /// MED-S6: SAN must be `iPAddress` for an IP literal, with a 90-day
    /// validity window.
    #[test]
    fn test_generated_cert_has_ip_san_and_90d_validity() {
        let (cert_pem, _) = generate_self_signed("127.0.0.1").unwrap();
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let der = certs[0].as_ref();

        let (_, x509) = X509Certificate::from_der(der).expect("valid X.509 DER");

        // Validity window must be exactly 90 days from `not_before` to
        // `not_after` (give-or-take a couple of seconds for the OffsetDateTime
        // arithmetic — we built it as `now - 1h` and `now + 90d`).
        let nb = x509.validity().not_before.timestamp();
        let na = x509.validity().not_after.timestamp();
        let span_days = (na - nb) / 86_400;
        assert_eq!(
            span_days, 90,
            "validity span must be 90 days, got {span_days}"
        );

        // SAN must contain the iPAddress entry, not a DnsName.
        let san_ext = x509
            .extensions()
            .iter()
            .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME)
            .expect("certificate must have a SubjectAlternativeName extension");
        if let ParsedExtension::SubjectAlternativeName(san) = san_ext.parsed_extension() {
            let mut found_ip = false;
            for entry in &san.general_names {
                if let GeneralName::IPAddress(_) = entry {
                    found_ip = true;
                }
            }
            assert!(
                found_ip,
                "SAN must contain an iPAddress entry (got {san:?})"
            );
        } else {
            panic!("SubjectAlternativeName extension failed to parse");
        }
    }

    /// MED-S6: hostname (non-IP) input gets a `dNSName` SAN.
    #[test]
    fn test_generated_cert_has_dns_san_for_hostname() {
        let (cert_pem, _) = generate_self_signed("muxtop.example.com").unwrap();
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let (_, x509) = X509Certificate::from_der(certs[0].as_ref()).unwrap();
        let san_ext = x509
            .extensions()
            .iter()
            .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME)
            .expect("certificate must have a SAN extension");
        if let ParsedExtension::SubjectAlternativeName(san) = san_ext.parsed_extension() {
            let mut found_dns = false;
            for entry in &san.general_names {
                if let GeneralName::DNSName(name) = entry
                    && *name == "muxtop.example.com"
                {
                    found_dns = true;
                }
            }
            assert!(found_dns, "SAN must contain dNSName for hostname input");
        } else {
            panic!("SAN extension failed to parse");
        }
    }

    /// LOW-S1: server config is constructed by `acceptor_from_pem` without
    /// panicking and the only acceptable protocol version is TLS 1.3. We
    /// verify by trying to handshake with a TLS 1.2-only client and asserting
    /// that the connection fails (the server rejects it).
    #[tokio::test]
    async fn test_tls13_only_rejects_tls12_client() {
        use tokio::io::AsyncWriteExt;
        use tokio_rustls::rustls::{ClientConfig, RootCertStore};

        let (cert_pem, key_pem) = generate_self_signed("127.0.0.1").unwrap();

        let mut cert_file = NamedTempFile::new().unwrap();
        cert_file.write_all(cert_pem.as_bytes()).unwrap();
        let mut key_file = NamedTempFile::new().unwrap();
        key_file.write_all(key_pem.as_bytes()).unwrap();
        let acceptor = acceptor_from_pem(cert_file.path(), key_file.path()).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server: accept one connection then drop.
        let server_handle = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(sock).await; // expected: handshake error
        });

        // Client: pin to TLS 1.2 ONLY.
        let mut roots = RootCertStore::empty();
        let server_certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        for c in server_certs {
            roots.add(c).unwrap();
        }
        let client_cfg = ClientConfig::builder_with_protocol_versions(&[&version::TLS12])
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_cfg));
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let server_name = rustls_pki_types::ServerName::try_from("127.0.0.1").unwrap();

        // Either the connect call returns an error (the common case), or the
        // handshake completes a half-broken socket that fails on first I/O.
        // Both are acceptable evidence that TLS 1.2 is rejected.
        let result = connector.connect(server_name, tcp).await;
        match result {
            Err(_) => {} // expected
            Ok(mut s) => {
                let err = s.write_all(b"x").await;
                let flush = s.flush().await;
                assert!(
                    err.is_err() || flush.is_err(),
                    "TLS 1.2 client must not be able to talk to a TLS 1.3-only server"
                );
            }
        }
        let _ = server_handle.await;
    }
}
