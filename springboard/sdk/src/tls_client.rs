//! TLS handshake options applied to a client connection: `Network` parity for `NWProtocolTLS` and
//! `sec_protocol_options_t`.
//!
//! The options model is `bifrost_ui::tls_options::TlsOptions`, which checks the set before anything is built.
//! [`client_config`] turns it into the rustls client configuration the shell's HTTP client uses (the same
//! graviola provider and webpki roots), and [`connect`] starts a TLS session over a TCP stream with the
//! options' server name. The protocol range limits the versions offered, the application protocol names go
//! into the handshake, and the trust set either checks the chain against the roots or accepts only a peer
//! certificate whose SHA-256 digest is pinned (the chain is then not checked, and the handshake signature is).

use crate::digest::sha256;
use bifrost_ui::tls_options::{TlsOptions, TlsVersion, Trust};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use std::io;
use std::net::TcpStream;
use std::sync::Arc;

/// Builds the rustls client configuration for `options`. Callers that connect often may keep the result.
pub fn client_config(options: &TlsOptions) -> Result<ClientConfig, rustls::Error> {
    let provider = Arc::new(rustls_graviola::default_provider());
    let mut versions: Vec<&'static rustls::SupportedProtocolVersion> = Vec::new();
    if options.allows(TlsVersion::Tls12) {
        versions.push(&rustls::version::TLS12);
    }
    if options.allows(TlsVersion::Tls13) {
        versions.push(&rustls::version::TLS13);
    }
    let builder = ClientConfig::builder_with_provider(provider.clone()).with_protocol_versions(&versions)?;
    let mut config = match options.trust() {
        Trust::SystemRoots => {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        Trust::Pinned(pins) => {
            let verifier = Arc::new(PinVerifier { pins: pins.clone(), provider });
            builder.dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth()
        }
    };
    config.alpn_protocols = options.alpn().iter().map(|p| p.as_bytes().to_vec()).collect();
    Ok(config)
}

/// Starts a TLS client session over `tcp` with `options`. The handshake runs on the first read or write, as
/// with any rustls stream. The server name is the options' name when one is set, otherwise `host`.
pub fn connect(options: &TlsOptions, host: &str, tcp: TcpStream) -> io::Result<StreamOwned<ClientConnection, TcpStream>> {
    let name = options.server_name().unwrap_or(host);
    let name = ServerName::try_from(name.to_string()).map_err(|_| io::Error::other("bad server name"))?;
    let config = client_config(options).map_err(io::Error::other)?;
    let conn = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    Ok(StreamOwned::new(conn, tcp))
}

/// Accepts a peer whose end-entity certificate digest is pinned. Signatures are still checked by the provider.
#[derive(Debug)]
struct PinVerifier {
    pins: Vec<[u8; 32]>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self.pins.contains(&sha256(end_entity.as_ref())) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("the peer certificate is not one of the pinned certificates".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_ui::tls_options::TlsOptionsError;

    fn pin_verifier(pins: Vec<[u8; 32]>) -> PinVerifier {
        PinVerifier { pins, provider: Arc::new(rustls_graviola::default_provider()) }
    }

    #[test]
    fn default_options_build_a_config_with_no_protocols() {
        let config = client_config(&TlsOptions::default()).unwrap();
        assert!(config.alpn_protocols.is_empty());
    }

    #[test]
    fn application_protocols_go_into_the_handshake_in_order() {
        let options = TlsOptions::default().with_alpn(&["h2", "http/1.1"]).unwrap();
        let config = client_config(&options).unwrap();
        assert_eq!(config.alpn_protocols, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
    }

    #[test]
    fn a_single_version_range_builds() {
        let options = TlsOptions::default().with_versions(TlsVersion::Tls13, TlsVersion::Tls13).unwrap();
        assert!(client_config(&options).is_ok());
    }

    #[test]
    fn an_empty_pin_set_is_refused_before_a_config_exists() {
        assert_eq!(TlsOptions::default().with_trust(Trust::Pinned(Vec::new())).err(), Some(TlsOptionsError::EmptyPinSet));
    }

    #[test]
    fn a_pinned_config_builds() {
        let options = TlsOptions::default().with_trust(Trust::Pinned(vec![[7u8; 32]])).unwrap();
        assert!(client_config(&options).is_ok());
    }

    #[test]
    fn a_pinned_certificate_is_accepted_and_others_are_refused() {
        let der = b"certificate bytes".to_vec();
        let verifier = pin_verifier(vec![sha256(&der)]);
        let name = ServerName::try_from("example.com").unwrap();
        let now = UnixTime::now();
        let ok = verifier.verify_server_cert(&CertificateDer::from(der), &[], &name, &[], now);
        assert!(ok.is_ok());
        let other = CertificateDer::from(b"another certificate".to_vec());
        assert!(verifier.verify_server_cert(&other, &[], &name, &[], now).is_err());
    }

    #[test]
    fn a_bad_server_name_is_an_error() {
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| {
            let addr = l.local_addr()?;
            TcpStream::connect(addr)
        });
        if let Ok(stream) = tcp {
            assert!(connect(&TlsOptions::default(), "", stream).is_err());
        }
    }
}
