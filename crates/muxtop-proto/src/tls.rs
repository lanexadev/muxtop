// TLS client configuration for muxtop.

use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls_pki_types::CertificateDer;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

/// TLS client configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum TlsClientError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("no certificates found in CA file")]
    NoCertificates,

    #[error("TLS configuration error: {0}")]
    Rustls(#[from] tokio_rustls::rustls::Error),
}

/// Build a `TlsConnector` that trusts certificates from a PEM-encoded CA file.
pub fn connector_from_ca(ca_path: &Path) -> Result<TlsConnector, TlsClientError> {
    let ca_file = std::fs::File::open(ca_path)?;
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(ca_file))
            .collect::<Result<Vec<_>, _>>()?;

    if certs.is_empty() {
        return Err(TlsClientError::NoCertificates);
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store
            .add(cert)
            .map_err(TlsClientError::Rustls)?;
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a `TlsConnector` that skips certificate verification (INSECURE — for development only).
pub fn connector_insecure() -> TlsConnector {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerifier))
        .with_no_client_auth();

    TlsConnector::from(Arc::new(config))
}

/// A certificate verifier that accepts any certificate (INSECURE).
#[derive(Debug)]
struct NoVerifier;

impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: tokio_rustls::rustls::pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<tokio_rustls::rustls::client::danger::HandshakeSignatureValid, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_self_signed_cert() -> (String, String) {
        let san = vec!["localhost".to_string()];
        let ck = rcgen::generate_simple_self_signed(san).unwrap();
        (ck.cert.pem(), ck.signing_key.serialize_pem())
    }

    #[test]
    fn test_connector_from_ca_valid() {
        let (cert_pem, _) = make_self_signed_cert();
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(cert_pem.as_bytes()).unwrap();

        let connector = connector_from_ca(f.path());
        assert!(connector.is_ok());
    }

    #[test]
    fn test_connector_from_ca_missing_file() {
        let result = connector_from_ca(Path::new("/nonexistent/ca.pem"));
        assert!(result.is_err());
    }

    #[test]
    fn test_connector_insecure_builds() {
        let _connector = connector_insecure();
    }
}
