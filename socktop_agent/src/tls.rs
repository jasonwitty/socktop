use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, SanType};
use time::{Duration, OffsetDateTime};
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

    let mut params = CertificateParams::new(vec![hostname.clone(), "localhost".into()])?;
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    params.subject_alt_names.push(SanType::IpAddress(IpAddr::V6(
        ::std::net::Ipv6Addr::LOCALHOST,
    )));
    params
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, hostname.clone());
    params.distinguished_name = dn;
    params.is_ca = IsCa::NoCa;
    // Dynamic validity: start slightly in the past to avoid clock skew issues, end ~397 days later
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::minutes(5);
    params.not_after = now + Duration::days(397);

    // Generate key pair (default is ECDSA P256 SHA256)
    let key_pair = rcgen::KeyPair::generate()?; // defaults to ECDSA P256 SHA256
    let cert = params.self_signed(&key_pair)?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

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
