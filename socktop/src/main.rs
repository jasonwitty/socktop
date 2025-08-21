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
    demo: bool,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParsedArgs, String> {
    let mut it = args.into_iter();
    let prog = it.next().unwrap_or_else(|| "socktop".into());
    let mut url: Option<String> = None;
    let mut tls_ca: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut save = false; // --save
    let mut demo = false; // --demo

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err(format!(
                    "Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [--demo] [ws://HOST:PORT/ws]" 
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
            "--demo" => {
                demo = true;
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
                        "Unexpected argument. Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [--demo] [ws://HOST:PORT/ws]"
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
        demo,
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

    // Demo mode short-circuit (ignore other args except conflicting ones)
    if parsed.demo || matches!(parsed.profile.as_deref(), Some("demo")) {
        return run_demo_mode(parsed.tls_ca.as_deref()).await;
    }

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
                                profiles_mut.profiles.insert(
                                    name.clone(),
                                    ProfileEntry {
                                        url: u.clone(),
                                        tls_ca: t.clone(),
                                    },
                                );
                                let _ = save_profiles(&profiles_mut);
                            }
                        }
                    }
                }
            }
            (u, t)
        }
        ResolveProfile::Loaded(u, t) => (u, t),
        ResolveProfile::PromptSelect(mut names) => {
            // Always add demo option to list
            if !names.iter().any(|n| n == "demo") { names.push("demo".into()); }
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
                        if name == "demo" { return run_demo_mode(parsed.tls_ca.as_deref()).await; }
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

// --- Demo Mode ---

async fn run_demo_mode(_tls_ca: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let port = 3231;
    let url = format!("ws://127.0.0.1:{port}/ws");
    let child = spawn_demo_agent(port)?;
    // Use select to handle Ctrl-C and normal quit
    let mut app = App::new();
    tokio::select! {
        res = app.run(&url, None) => { drop(child); res }
        _ = tokio::signal::ctrl_c() => {
            // Drop child (kills agent) then return
            drop(child);
            Ok(())
        }
    }
}

struct DemoGuard(std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>);
impl Drop for DemoGuard { fn drop(&mut self) { if let Some(mut ch) = self.0.lock().unwrap().take() { let _ = ch.kill(); } } }

fn spawn_demo_agent(port: u16) -> Result<DemoGuard, Box<dyn std::error::Error>> {
    let candidate = find_agent_executable();
    let mut cmd = std::process::Command::new(candidate);
    cmd.arg("--port").arg(port.to_string());
    cmd.env("SOCKTOP_ENABLE_SSL", "0");
    cmd.env("SOCKTOP_AGENT_GPU", "0");
    cmd.env("SOCKTOP_AGENT_TEMP", "0");
    let child = cmd.spawn()?;
    // Give the agent a brief moment to start
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(DemoGuard(std::sync::Arc::new(std::sync::Mutex::new(Some(child)))))
}

fn find_agent_executable() -> std::path::PathBuf {
    let self_exe = std::env::current_exe().ok();
    if let Some(exe) = self_exe {
        if let Some(parent) = exe.parent() {
            #[cfg(windows)]
            let name = "socktop_agent.exe";
            #[cfg(not(windows))]
            let name = "socktop_agent";
            let candidate = parent.join(name);
            if candidate.exists() { return candidate; }
        }
    }
    // Fallback to relying on PATH
    std::path::PathBuf::from("socktop_agent")
}
