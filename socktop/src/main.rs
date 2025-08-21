//! Entry point for the socktop TUI. Parses args and runs the App.

mod app;
mod history;
mod profiles;
mod types;
mod ui;
mod ws;

use app::App;
use profiles::{load_profiles, save_profiles, ProfileEntry, ProfileRequest, ResolveProfile};
use std::env;
use std::io::{self, Write};

struct ParsedArgs {
    url: Option<String>,
    tls_ca: Option<String>,
    profile: Option<String>,
    save: bool,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParsedArgs, String> {
    let mut it = args.into_iter();
    let prog = it.next().unwrap_or_else(|| "socktop".into());
    let mut url: Option<String> = None;
    let mut tls_ca: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut save = false; // --save-profile

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err(format!(
                    "Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [ws://HOST:PORT/ws]" 
                ));
            }
            "--tls-ca" | "-t" => {
                tls_ca = it.next();
            }
            "--profile" | "-P" => {
                profile = it.next();
            }
            "--save" => {
                save = true;
            }
            _ if arg.starts_with("--tls-ca=") => {
                if let Some((_, v)) = arg.split_once('=') {
                    if !v.is_empty() {
                        tls_ca = Some(v.to_string());
                    }
                }
            }
            _ if arg.starts_with("--profile=") => {
                if let Some((_, v)) = arg.split_once('=') {
                    if !v.is_empty() {
                        profile = Some(v.to_string());
                    }
                }
            }
            _ => {
                if url.is_none() {
                    url = Some(arg);
                } else {
                    return Err(format!(
                        "Unexpected argument. Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [ws://HOST:PORT/ws]"
                    ));
                }
            }
        }
    }
    Ok(ParsedArgs {
        url,
        tls_ca,
        profile,
        save,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reuse the same parsing logic for testability
    let parsed = match parse_args(env::args()) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return Ok(());
        }
    };

    let profiles_file = load_profiles();
    let req = ProfileRequest {
        profile_name: parsed.profile.clone(),
        url: parsed.url.clone(),
        tls_ca: parsed.tls_ca.clone(),
    };
    let resolved = req.resolve(&profiles_file);

    // Determine final connection parameters (and maybe mutated profiles to persist)
    let mut profiles_mut = profiles_file.clone();
    let (url, tls_ca): (String, Option<String>) = match resolved {
        ResolveProfile::Direct(u, t) => {
            // Possibly save if profile specified and --save or new entry
            if let Some(name) = parsed.profile.as_ref() {
                let existing = profiles_mut.profiles.get(name);
                match existing {
                    None => {
                        // New profile: auto-save immediately
                        profiles_mut.profiles.insert(
                            name.clone(),
                            ProfileEntry {
                                url: u.clone(),
                                tls_ca: t.clone(),
                            },
                        );
                        let _ = save_profiles(&profiles_mut);
                    }
                    Some(entry) => {
                        let changed = entry.url != u || entry.tls_ca != t;
                        if changed {
                            let overwrite = if parsed.save {
                                true
                            } else {
                                prompt_yes_no(&format!(
                                    "Overwrite existing profile '{name}'? [y/N]: "
                                ))
                            };
                            if overwrite {
                                profiles_mut.profiles.insert(name.clone(), ProfileEntry { url: u.clone(), tls_ca: t.clone() });
                                let _ = save_profiles(&profiles_mut);
                            }
                        }
                    }
                }
            }
            (u, t)
        }
        ResolveProfile::Loaded(u, t) => (u, t),
        ResolveProfile::PromptSelect(names) => {
            eprintln!("Select profile:");
            for (i, n) in names.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, n);
            }
            eprint!("Enter number (or blank to abort): ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            if io::stdin().read_line(&mut line).is_ok() {
                if let Ok(idx) = line.trim().parse::<usize>() {
                    if idx >= 1 && idx <= names.len() {
                        let name = &names[idx - 1];
                        if let Some(entry) = profiles_mut.profiles.get(name) {
                            (entry.url.clone(), entry.tls_ca.clone())
                        } else {
                            return Ok(());
                        }
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        }
        ResolveProfile::PromptCreate(name) => {
            eprintln!("Profile '{name}' does not exist yet.");
            let url = prompt_string("Enter URL (ws://HOST:PORT/ws or wss://...): ")?;
            if url.trim().is_empty() {
                return Ok(());
            }
            let ca = prompt_string("Enter TLS CA path (or leave blank): ")?;
            let ca_opt = if ca.trim().is_empty() {
                None
            } else {
                Some(ca.trim().to_string())
            };
            profiles_mut.profiles.insert(
                name.clone(),
                ProfileEntry {
                    url: url.trim().to_string(),
                    tls_ca: ca_opt.clone(),
                },
            );
            let _ = save_profiles(&profiles_mut);
            (url.trim().to_string(), ca_opt)
        }
        ResolveProfile::None => {
            eprintln!("No URL provided and no profiles to select.");
            return Ok(());
        }
    };

    let mut app = App::new();
    app.run(&url, tls_ca.as_deref()).await
}

fn prompt_yes_no(prompt: &str) -> bool {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_ok() {
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    } else {
        false
    }
}

fn prompt_string(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
}
