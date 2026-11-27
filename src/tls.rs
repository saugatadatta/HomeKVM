//! TLS 1.3 (inside QUIC) between HomeKVM PCs.
//!
//! Both sides present their self-signed certificate (mutual TLS). The TLS layer
//! checks that each side really holds the key for the certificate it shows (the
//! handshake signature), but doesn't judge *who* it is: there is no certificate
//! authority. That decision happens right after the handshake: the peer's
//! certificate fingerprint is its ID, and only IDs the user paired (after
//! comparing a 6-digit code) are trusted.

use crate::identity::Identity;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::sync::Arc;
use std::time::Duration;

pub const ALPN: &[u8] = b"homekvm/1";

#[derive(Debug)]
struct PeerCert(Arc<CryptoProvider>);

impl ServerCertVerifier for PeerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Identity is checked by fingerprint after the handshake (see module docs).
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for PeerCert {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        // Identity is checked by fingerprint after the handshake (see module docs).
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    // Notice a PC that went away within ~15 s; keep idle links open.
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    t.max_idle_timeout(Some(quinn::IdleTimeout::try_from(Duration::from_secs(15)).expect("valid timeout")));
    Arc::new(t)
}

pub fn server(me: &Identity) -> Result<quinn::ServerConfig, String> {
    let p = provider();
    let mut tls = rustls::ServerConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_client_cert_verifier(Arc::new(PeerCert(p)))
        .with_single_cert(vec![me.cert.clone()], me.key())
        .map_err(|e| e.to_string())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    config.transport_config(transport());
    Ok(config)
}

pub fn client(me: &Identity) -> Result<quinn::ClientConfig, String> {
    let p = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PeerCert(p)))
        .with_client_auth_cert(vec![me.cert.clone()], me.key())
        .map_err(|e| e.to_string())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| e.to_string())?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(transport());
    Ok(config)
}

/// The peer's ID: the fingerprint of the certificate it proved it owns.
pub fn peer_id(conn: &quinn::Connection) -> Option<String> {
    let any = conn.peer_identity()?;
    let certs = any.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| crate::identity::fingerprint(c.as_ref()))
}

/// The 6-digit pairing code. Both PCs derive it from their shared TLS session, so a
/// device sitting in the middle would get a different session, and a different code,
/// on each side.
pub fn pairing_code(conn: &quinn::Connection, a: &str, b: &str) -> Option<String> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let mut out = [0u8; 8];
    conn.export_keying_material(&mut out, b"homekvm pairing code", format!("{lo}{hi}").as_bytes()).ok()?;
    let n = u64::from_be_bytes(out) % 1_000_000;
    Some(format!("{:03} {:03}", n / 1000, n % 1000))
}
