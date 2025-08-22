use rcgen::{Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, SanType};
use std::{
    fs,
    io::Write,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("socktop_agent")
        .join("tls")
}

pub fn cert_paths() -> (PathBuf, PathBuf) {
    let dir = config_dir();
    (dir.join("cert.pem"), dir.join("key.pem"))
}

pub fn ensure_self_signed_cert() -> anyhow::Result<(PathBuf, PathBuf)> {
    let (cert_path, key_path) = cert_paths();
    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path, key_path));
    }
    fs::create_dir_all(cert_path.parent().unwrap())?;

    let hostname = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    let mut params = CertificateParams::new(vec![hostname.clone(), "localhost".into()]);
    // Add IP SANs
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V6(::std::net::Ipv6Addr::LOCALHOST)));
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));

    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, hostname.clone());
    params.is_ca = IsCa::NoCa;
    // 397 days like previous implementation
    params.not_before = rcgen::date_time_ymd(2024, 1, 1); // stable starting point
    params.not_after = params.not_before + rcgen::PKCS_EPOCH_DURATION * 0; // overwritten below
    // rcgen doesn't allow direct relative days for not_after while keeping not_before now; use validity_days
    params.validity_days = 397;

    // Use modern defaults (Ed25519) for key; fallback to RSA if necessary
    // Keep RSA to maximize compatibility with older clients
    params.alg = &rcgen::PKCS_ECDSA_P256_SHA256; // widely supported
    let cert = Certificate::from_params(params)?;
    let cert_pem = cert.serialize_pem()?;
    let key_pem = cert.serialize_private_key_pem();

    let mut f = fs::File::create(&cert_path)?;
    f.write_all(cert_pem.as_bytes())?;
    let mut k = fs::File::create(&key_path)?;
    k.write_all(key_pem.as_bytes())?;

    println!(
        "socktop_agent: generated self-signed TLS certificate at {}",
        cert_path.display()
    );
    println!("socktop_agent: private key at {}", key_path.display());
    Ok((cert_path, key_path))
}
