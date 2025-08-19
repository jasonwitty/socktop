use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::x509::extension::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName,
};
use openssl::x509::{X509NameBuilder, X509};
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

    // Key
    let rsa = Rsa::generate(4096)?;
    let pkey = PKey::from_rsa(rsa)?;

    // Subject/issuer
    let hostname = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    let mut name = X509NameBuilder::new()?;
    name.append_entry_by_nid(Nid::COMMONNAME, &hostname)?;
    let name = name.build();

    // Cert builder
    let mut builder = X509::builder()?;
    builder.set_version(2)?;
    builder.set_subject_name(&name)?;
    builder.set_issuer_name(&name)?;
    builder.set_pubkey(&pkey)?;

    builder.set_not_before(Asn1Time::days_from_now(0)?.as_ref())?;
    builder.set_not_after(Asn1Time::days_from_now(397)?.as_ref())?;

    // SANs: hostname + localhost loopbacks
    let mut san = SubjectAlternativeName::new();
    san.dns(&hostname)
        .dns("localhost")
        .ip("127.0.0.1")
        .ip("::1");
    // Add a generic 0.0.0.0 for convenience; some TLS libs ignore this, but harmless.
    let _ = san.ip(&IpAddr::V4(Ipv4Addr::UNSPECIFIED).to_string());
    let san = san.build(&builder.x509v3_context(None, None))?;
    // End-entity cert: not a CA
    builder.append_extension(BasicConstraints::new().critical().build()?)?;
    builder.append_extension(
        KeyUsage::new()
            .digital_signature()
            .key_encipherment()
            .build()?,
    )?;
    // TLS server usage
    builder.append_extension(ExtendedKeyUsage::new().server_auth().build()?)?;
    builder.append_extension(san)?;

    builder.sign(&pkey, MessageDigest::sha256())?;
    let cert: X509 = builder.build();

    let mut f = fs::File::create(&cert_path)?;
    f.write_all(&cert.to_pem()?)?;
    let mut k = fs::File::create(&key_path)?;
    k.write_all(&pkey.private_key_to_pem_pkcs8()?)?;

    println!(
        "socktop_agent: generated self-signed TLS certificate at {}",
        cert_path.display()
    );
    println!("socktop_agent: private key at {}", key_path.display());
    Ok((cert_path, key_path))
}
