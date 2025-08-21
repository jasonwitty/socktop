//! Connection profiles: load/save simple JSON mapping of profile name -> { url, tls_ca }
//! Stored under XDG config dir: $XDG_CONFIG_HOME/socktop/profiles.json (fallback ~/.config/socktop/profiles.json)

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileEntry {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_ca: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfilesFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileEntry>,
    #[serde(default)]
    pub version: u32,
}

pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("socktop")
    } else {
        dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("socktop")
    }
}

pub fn profiles_path() -> PathBuf {
    config_dir().join("profiles.json")
}

pub fn load_profiles() -> ProfilesFile {
    let path = profiles_path();
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => ProfilesFile::default(),
    }
}

pub fn save_profiles(p: &ProfilesFile) -> std::io::Result<()> {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(p).expect("serialize profiles");
    fs::write(path, data)
}

pub enum ResolveProfile {
    /// Use the provided runtime inputs (not persisted). (url, tls_ca)
    Direct(String, Option<String>),
    /// Loaded from existing profile entry (url, tls_ca)
    Loaded(String, Option<String>),
    /// Should prompt user to select among profile names
    PromptSelect(Vec<String>),
    /// Should prompt user to create a new profile (name)
    PromptCreate(String),
    /// No profile could be resolved (e.g., missing arguments)
    None,
}

pub struct ProfileRequest {
    pub profile_name: Option<String>,
    pub url: Option<String>,
    pub tls_ca: Option<String>,
}

impl ProfileRequest {
    pub fn resolve(self, pf: &ProfilesFile) -> ResolveProfile {
        // Case: only profile name given -> try load
        if self.url.is_none() && self.profile_name.is_some() {
            let name = self.profile_name.unwrap();
            if let Some(entry) = pf.profiles.get(&name) {
                return ResolveProfile::Loaded(entry.url.clone(), entry.tls_ca.clone());
            } else {
                return ResolveProfile::PromptCreate(name);
            }
        }
        // Both provided -> direct (maybe later saved by caller)
        if let Some(u) = self.url {
            return ResolveProfile::Direct(u, self.tls_ca);
        }
        // Nothing provided -> maybe prompt select if profiles exist
        if self.url.is_none() && self.profile_name.is_none() {
            if pf.profiles.is_empty() {
                ResolveProfile::None
            } else {
                ResolveProfile::PromptSelect(pf.profiles.keys().cloned().collect())
            }
        } else {
            ResolveProfile::None
        }
    }
}
