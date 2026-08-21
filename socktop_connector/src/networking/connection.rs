//! WebSocket connection handling for native (non-WASM) environments.

use crate::config::ConnectorConfig;
use crate::error::{ConnectorError, Result};

use std::io::BufReader;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

#[cfg(feature = "tls")]
use {
    rustls::{self, ClientConfig},
    rustls::{
        DigitallySignedStruct, RootCertStore, SignatureScheme,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        crypto::{WebPkiSupportedAlgorithms, ring},
        pki_types::{CertificateDer, ServerName, UnixTime},
    },
    rustls_pemfile::Item,
    std::fs::File,
    tokio_tungstenite::Connector,
};

pub type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect to the agent and return the WS stream
pub async fn connect_to_agent(config: &ConnectorConfig) -> Result<WsStream> {
    #[cfg(feature = "tls")]
    ensure_crypto_provider();

    let mut u = Url::parse(&config.url)?;
    if let Some(ca_path) = &config.tls_ca_path {
        if u.scheme() == "ws" {
            let _ = u.set_scheme("wss");
        }
        return connect_with_ca_and_config(u.as_str(), ca_path, config).await;
    }
    // No TLS - hostname verification is not applicable
    connect_without_ca_and_config(u.as_str(), config).await
}

async fn connect_without_ca_and_config(url: &str, config: &ConnectorConfig) -> Result<WsStream> {
    let mut req = url.into_client_request()?;

    // Apply WebSocket protocol configuration
    if let Some(version) = &config.ws_version {
        req.headers_mut().insert(
            "Sec-WebSocket-Version",
            version
                .parse()
                .map_err(|_| ConnectorError::protocol_error("Invalid WebSocket version"))?,
        );
    }

    if let Some(protocols) = &config.ws_protocols {
        let protocols_str = protocols.join(", ");
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocols_str
                .parse()
                .map_err(|_| ConnectorError::protocol_error("Invalid WebSocket protocols"))?,
        );
    }

    // `true` disables Nagle: small request/response frames, latency matters.
    let (ws, _) = tokio_tungstenite::connect_async_with_config(req, None, true).await?;
    Ok(ws)
}

#[cfg(feature = "tls")]
async fn connect_with_ca_and_config(
    url: &str,
    ca_path: &str,
    config: &ConnectorConfig,
) -> Result<WsStream> {
    // Initialize the crypto provider for rustls
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut root = RootCertStore::empty();
    let mut reader = BufReader::new(File::open(ca_path)?);
    let mut der_certs = Vec::new();
    while let Ok(Some(item)) = rustls_pemfile::read_one(&mut reader) {
        if let Item::X509Certificate(der) = item {
            der_certs.push(der);
        }
    }
    if der_certs.is_empty() {
        return Err(ConnectorError::protocol_error(format!(
            "no certificates found in --tls-ca file: {ca_path}"
        )));
    }
    root.add_parsable_certificates(der_certs.iter().cloned());

    let mut cfg = ClientConfig::builder()
        .with_root_certificates(root)
        .with_no_client_auth();

    let mut req = url.into_client_request()?;

    // Apply WebSocket protocol configuration
    if let Some(version) = &config.ws_version {
        req.headers_mut().insert(
            "Sec-WebSocket-Version",
            version
                .parse()
                .map_err(|_| ConnectorError::protocol_error("Invalid WebSocket version"))?,
        );
    }

    if let Some(protocols) = &config.ws_protocols {
        let protocols_str = protocols.join(", ");
        req.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            protocols_str
                .parse()
                .map_err(|_| ConnectorError::protocol_error("Invalid WebSocket protocols"))?,
        );
    }

    if !config.verify_hostname {
        // Default mode: certificate PINNING without hostname verification.
        // The server must present a certificate byte-identical to one in the
        // --tls-ca file. This intentionally ignores expiry and chain building
        // (the operator pinned this exact cert), but unlike a blanket accept
        // it makes MITM certs fail the handshake.
        cfg.dangerous()
            .set_certificate_verifier(Arc::new(PinnedCertVerifier::new(der_certs)));
    }
    let cfg = Arc::new(cfg);
    // Third argument is tungstenite's `disable_nagle`: always true — socktop
    // exchanges small request/response frames where Nagle only adds latency.
    let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(
        req,
        None,
        true,
        Some(Connector::Rustls(cfg)),
    )
    .await?;
    Ok(ws)
}

/// Accepts exactly the certificates the user pinned via `--tls-ca`, nothing else.
///
/// Used when hostname verification is off (the default for self-signed
/// home-lab certs). Signature validation still runs with the ring provider's
/// full algorithm set; only the certificate identity check is replaced —
/// by an exact DER comparison against the pinned certificate(s).
#[cfg(feature = "tls")]
#[derive(Debug)]
struct PinnedCertVerifier {
    pinned: Vec<CertificateDer<'static>>,
    algorithms: WebPkiSupportedAlgorithms,
}

#[cfg(feature = "tls")]
impl PinnedCertVerifier {
    fn new(pinned: Vec<CertificateDer<'static>>) -> Self {
        Self {
            pinned,
            algorithms: ring::default_provider().signature_verification_algorithms,
        }
    }
}

#[cfg(feature = "tls")]
impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if self.pinned.iter().any(|p| p == end_entity) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(not(feature = "tls"))]
async fn connect_with_ca_and_config(
    _url: &str,
    _ca_path: &str,
    _config: &ConnectorConfig,
) -> Result<WsStream> {
    Err(ConnectorError::tls_error(
        "TLS support not compiled in",
        std::io::Error::new(std::io::ErrorKind::Unsupported, "TLS not available"),
    ))
}

#[cfg(feature = "tls")]
fn ensure_crypto_provider() {
    let _ = ring::default_provider().install_default();
}

#[cfg(all(test, feature = "tls"))]
mod tests {
    use super::*;

    fn verifier(pinned: &[&[u8]]) -> PinnedCertVerifier {
        let _ = ring::default_provider().install_default();
        PinnedCertVerifier::new(
            pinned
                .iter()
                .map(|b| CertificateDer::from(b.to_vec()))
                .collect(),
        )
    }

    fn verify(v: &PinnedCertVerifier, presented: &[u8]) -> bool {
        v.verify_server_cert(
            &CertificateDer::from(presented.to_vec()),
            &[],
            &ServerName::try_from("agent.test").unwrap(),
            &[],
            UnixTime::now(),
        )
        .is_ok()
    }

    /// The regression this verifier exists to prevent: the old NoVerify
    /// accepted ANY certificate when hostname verification was off, so the
    /// documented pinning was a no-op. The pinned cert must be accepted and
    /// every other cert rejected.
    #[test]
    fn only_the_pinned_certificate_is_accepted() {
        let v = verifier(&[b"pinned-cert-der"]);
        assert!(verify(&v, b"pinned-cert-der"));
        assert!(!verify(&v, b"some-mitm-cert"), "unpinned cert accepted");
        assert!(!verify(&v, b""), "empty cert accepted");
    }

    /// A --tls-ca file may hold several certs (e.g. during rotation); any of
    /// them must satisfy the pin.
    #[test]
    fn any_cert_in_a_multi_cert_pem_satisfies_the_pin() {
        let v = verifier(&[b"old-cert", b"new-cert"]);
        assert!(verify(&v, b"old-cert"));
        assert!(verify(&v, b"new-cert"));
        assert!(!verify(&v, b"third-party-cert"));
    }

    /// Fail closed: an empty pin set must reject everything rather than
    /// falling back to accept-all.
    #[test]
    fn an_empty_pin_set_rejects_all_certificates() {
        let v = verifier(&[]);
        assert!(!verify(&v, b"anything"));
    }

    /// Signature schemes come from the real provider, not a hardcoded list —
    /// an agent using e.g. RSA-PKCS1 must still be able to handshake.
    #[test]
    fn signature_schemes_come_from_the_provider() {
        let v = verifier(&[b"x"]);
        let schemes = v.supported_verify_schemes();
        assert!(
            schemes.len() > 3,
            "suspiciously short scheme list: {schemes:?}"
        );
        assert!(schemes.contains(&SignatureScheme::RSA_PKCS1_SHA256));
        assert!(schemes.contains(&SignatureScheme::ECDSA_NISTP256_SHA256));
    }
}
