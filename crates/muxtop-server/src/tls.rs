// TLS configuration for muxtop-server.

use std::path::Path;
use std::sync::Arc;

use rcgen::generate_simple_self_signed;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;

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
pub fn acceptor_from_pem(cert_path: &Path, key_path: &Path) -> Result<TlsAcceptor, TlsError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|_| TlsError::NoCertificates)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::NoCertificates)?;

    if certs.is_empty() {
        return Err(TlsError::NoCertificates);
    }

    let key = PrivateKeyDer::from_pem_file(key_path).map_err(|_| TlsError::NoPrivateKey)?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Generate a self-signed certificate and private key.
///
/// Returns `(cert_pem, key_pem)` as PEM-encoded strings.
pub fn generate_self_signed(hostname: &str) -> Result<(String, String), TlsError> {
    let subject_alt_names = vec![hostname.to_string()];
    let certified_key = generate_simple_self_signed(subject_alt_names)?;

    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();

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
}
