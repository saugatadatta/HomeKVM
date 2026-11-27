//! This PC's identity: a key and self-signed certificate made on first start.
//! A PC's ID is the SHA-256 fingerprint of its certificate; pairing pins that ID.

use crate::profile;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub struct Identity {
    /// Hex SHA-256 of the certificate.
    pub id: String,
    pub cert: CertificateDer<'static>,
    key: Vec<u8>,
}

impl Identity {
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }
}

pub fn load_or_create() -> Result<Identity, String> {
    let dir = profile::data_dir().ok_or("APPDATA is not set")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cert_path = dir.join("identity.cert.der");
    let key_path = dir.join("identity.key.der");

    let (cert, key) = match (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        (Ok(c), Ok(k)) => (c, k),
        _ => {
            let made = rcgen::generate_simple_self_signed(vec!["homekvm".to_string()]).map_err(|e| e.to_string())?;
            let cert = made.cert.der().to_vec();
            let key = made.key_pair.serialize_der();
            std::fs::write(&key_path, &key).map_err(|e| e.to_string())?;
            std::fs::write(&cert_path, &cert).map_err(|e| e.to_string())?;
            (cert, key)
        }
    };
    Ok(Identity { id: fingerprint(&cert), cert: CertificateDer::from(cert), key })
}

pub fn fingerprint(der: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, der).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// "7F3A…C2D1" for showing an ID to people.
pub fn short(id: &str) -> String {
    if id.len() < 8 {
        return id.to_uppercase();
    }
    format!("{}…{}", &id[..4], &id[id.len() - 4..]).to_uppercase()
}
