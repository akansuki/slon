use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::config::{ConnectionConfig, SslMode};
use crate::error::{ErrorKind, PgError};

/// Accepts any server certificate while still checking handshake signatures.
/// This matches libpq's `sslmode=require` (encryption, no identity check), which is
/// what most self-signed development servers need.
#[derive(Debug)]
struct AcceptAnyCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// `verify-ca`: the chain must be trusted, but the host name is not compared with the
/// certificate (libpq semantics). Everything else is delegated to the real verifier.
#[derive(Debug)]
struct CaOnly(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for CaOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(TlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn tls_error(message: String) -> PgError {
    PgError::new(ErrorKind::Connection, message)
}

/// The CA certificates to trust: the PEM file if one is configured, otherwise the
/// operating system's store.
fn load_roots(file: Option<&str>) -> Result<RootCertStore, PgError> {
    let mut roots = RootCertStore::empty();
    match file.map(str::trim).filter(|f| !f.is_empty()) {
        Some(path) => {
            let certs = CertificateDer::pem_file_iter(path)
                .map_err(|e| {
                    tls_error(format!(
                        "Could not read the CA certificate file '{path}': {e}"
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    tls_error(format!(
                        "The CA certificate file '{path}' is not valid PEM: {e}"
                    ))
                })?;
            let (added, _ignored) = roots.add_parsable_certificates(certs);
            if added == 0 {
                return Err(tls_error(format!(
                    "The CA certificate file '{path}' contains no usable certificates. \
                     Expected PEM text starting with -----BEGIN CERTIFICATE-----."
                )));
            }
        }
        None => {
            let native = rustls_native_certs::load_native_certs();
            let (added, _ignored) = roots.add_parsable_certificates(native.certs);
            if added == 0 {
                return Err(tls_error(
                    "No trusted CA certificates were found on this system. \
                     Choose a CA certificate file for this connection."
                        .to_string(),
                ));
            }
        }
    }
    Ok(roots)
}

/// TLS settings for `cfg`: encryption only for `prefer`/`require`, certificate checks for
/// `verify-ca`/`verify-full`.
pub(crate) fn make_connector(cfg: &ConnectionConfig) -> Result<MakeRustlsConnect, PgError> {
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions");

    let config = if cfg.ssl_mode.verifies() {
        let roots = load_roots(cfg.ssl_root_cert.as_deref())?;
        let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|e| tls_error(format!("Could not set up certificate verification: {e}")))?;
        if cfg.ssl_mode == SslMode::VerifyFull {
            builder.with_webpki_verifier(verifier).with_no_client_auth()
        } else {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(CaOnly(verifier)))
                .with_no_client_auth()
        }
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider)))
            .with_no_client_auth()
    };
    Ok(MakeRustlsConnect::new(config))
}

/// Turn rustls' terse certificate errors into something a person can act on.
pub(crate) fn explain_tls_failure(err: &mut PgError, cfg: &ConnectionConfig) {
    if !cfg.ssl_mode.verifies() || err.hint.is_some() {
        return;
    }
    let m = err.message.to_lowercase();
    let hint = if m.contains("unknownissuer") || m.contains("unknown issuer") {
        Some("The server's certificate is not signed by a CA you trust. Choose the CA certificate file that signed it, or use 'Require' to encrypt without verifying.".to_string())
    } else if m.contains("not valid for name") || m.contains("notvalidforname") {
        Some(format!(
            "The certificate was not issued for '{}'. Connect with a host name listed in the certificate, or use 'Verify CA' to skip the host name check.",
            cfg.host.trim()
        ))
    } else if m.contains("expired") {
        Some("The server's certificate has expired.".to_string())
    } else if m.contains("not valid yet") || m.contains("notvalidyet") {
        Some("The server's certificate is not valid yet. Check this computer's clock.".to_string())
    } else if m.contains("server does not support tls") {
        Some(
            "This server does not offer TLS, and certificate verification requires it.".to_string(),
        )
    } else {
        None
    };
    err.hint = hint;
}
