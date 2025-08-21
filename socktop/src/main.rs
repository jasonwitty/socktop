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

pub(crate) struct ParsedArgs {
    url: Option<String>,
    tls_ca: Option<String>,
    profile: Option<String>,
    save: bool,
    demo: bool,
    dry_run: bool, // hidden test helper: skip connecting
    metrics_interval_ms: Option<u64>,
    processes_interval_ms: Option<u64>,
}

pub(crate) fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParsedArgs, String> {
    let mut it = args.into_iter();
    let prog = it.next().unwrap_or_else(|| "socktop".into());
    let mut url: Option<String> = None;
    let mut tls_ca: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut save = false;
    let mut demo = false;
    let mut dry_run = false;
    let mut metrics_interval_ms: Option<u64> = None;
    let mut processes_interval_ms: Option<u64> = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err(format!("Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [--demo] [--metrics-interval-ms N] [--processes-interval-ms N] [ws://HOST:PORT/ws]\n"));
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
            "--dry-run" => {
                // intentionally undocumented
                dry_run = true;
            }
            "--metrics-interval-ms" => {
                metrics_interval_ms = it.next().and_then(|v| v.parse().ok());
            }
            "--processes-interval-ms" => {
                processes_interval_ms = it.next().and_then(|v| v.parse().ok());
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
            _ if arg.starts_with("--metrics-interval-ms=") => {
                if let Some((_, v)) = arg.split_once('=') {
                    metrics_interval_ms = v.parse().ok();
                }
            }
            _ if arg.starts_with("--processes-interval-ms=") => {
                if let Some((_, v)) = arg.split_once('=') {
                    processes_interval_ms = v.parse().ok();
                }
            }
            _ => {
                if url.is_none() {
                    url = Some(arg);
                } else {
                    return Err(format!("Unexpected argument. Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] [--profile NAME|-P NAME] [--save] [--demo] [ws://HOST:PORT/ws]"));
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
        dry_run,
        metrics_interval_ms,
        processes_interval_ms,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = match parse_args(env::args()) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return Ok(());
        }
    };
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
    let mut profiles_mut = profiles_file.clone();
    let (url, tls_ca, metrics_interval_ms, processes_interval_ms): (
        String,
        Option<String>,
        Option<u64>,
        Option<u64>,
    ) = match resolved {
        ResolveProfile::Direct(u, t) => {
            if let Some(name) = parsed.profile.as_ref() {
                let existing = profiles_mut.profiles.get(name);
                match existing {
                    None => {
                        let (mi, pi) = gather_intervals(
                            parsed.metrics_interval_ms,
                            parsed.processes_interval_ms,
                        )?;
                        profiles_mut.profiles.insert(
                            name.clone(),
                            ProfileEntry {
                                url: u.clone(),
                                tls_ca: t.clone(),
                                metrics_interval_ms: mi,
                                processes_interval_ms: pi,
                            },
                        );
                        let _ = save_profiles(&profiles_mut);
                        (u, t, mi, pi)
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
                                let (mi, pi) = gather_intervals(
                                    parsed.metrics_interval_ms,
                                    parsed.processes_interval_ms,
                                )?;
                                profiles_mut.profiles.insert(
                                    name.clone(),
                                    ProfileEntry {
                                        url: u.clone(),
                                        tls_ca: t.clone(),
                                        metrics_interval_ms: mi,
                                        processes_interval_ms: pi,
                                    },
                                );
                                let _ = save_profiles(&profiles_mut);
                                (u, t, mi, pi)
                            } else {
                                (u, t, entry.metrics_interval_ms, entry.processes_interval_ms)
                            }
                        } else {
                            (u, t, entry.metrics_interval_ms, entry.processes_interval_ms)
                        }
                    }
                }
            } else {
                (
                    u,
                    t,
                    parsed.metrics_interval_ms,
                    parsed.processes_interval_ms,
                )
            }
        }
        ResolveProfile::Loaded(u, t) => {
            let entry = profiles_mut
                .profiles
                .get(parsed.profile.as_ref().unwrap())
                .unwrap();
            (u, t, entry.metrics_interval_ms, entry.processes_interval_ms)
        }
        ResolveProfile::PromptSelect(mut names) => {
            if !names.iter().any(|n| n == "demo") {
                names.push("demo".into());
            }
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
                        if name == "demo" {
                            return run_demo_mode(parsed.tls_ca.as_deref()).await;
                        }
                        if let Some(entry) = profiles_mut.profiles.get(name) {
                            (
                                entry.url.clone(),
                                entry.tls_ca.clone(),
                                entry.metrics_interval_ms,
                                entry.processes_interval_ms,
                            )
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
            let (mi, pi) =
                gather_intervals(parsed.metrics_interval_ms, parsed.processes_interval_ms)?;
            profiles_mut.profiles.insert(
                name.clone(),
                ProfileEntry {
                    url: url.trim().to_string(),
                    tls_ca: ca_opt.clone(),
                    metrics_interval_ms: mi,
                    processes_interval_ms: pi,
                },
            );
            let _ = save_profiles(&profiles_mut);
            (url.trim().to_string(), ca_opt, mi, pi)
        }
        ResolveProfile::None => {
            eprintln!("No URL provided and no profiles to select.");
            return Ok(());
        }
    };
    let mut app = App::new().with_intervals(metrics_interval_ms, processes_interval_ms);
    if parsed.dry_run {
        return Ok(());
    }
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

fn gather_intervals(
    arg_metrics: Option<u64>,
    arg_procs: Option<u64>,
) -> Result<(Option<u64>, Option<u64>), Box<dyn std::error::Error>> {
    let default_metrics = 500u64;
    let default_procs = 2000u64;
    let metrics = match arg_metrics {
        Some(v) => Some(v),
        None => {
            let inp = prompt_string(&format!(
                "Metrics interval ms (default {default_metrics}, Enter for default): "
            ))?;
            let t = inp.trim();
            if t.is_empty() {
                Some(default_metrics)
            } else {
                Some(t.parse()?)
            }
        }
    };
    let procs = match arg_procs {
        Some(v) => Some(v),
        None => {
            let inp = prompt_string(&format!(
                "Processes interval ms (default {default_procs}, Enter for default): "
            ))?;
            let t = inp.trim();
            if t.is_empty() {
                Some(default_procs)
            } else {
                Some(t.parse()?)
            }
        }
    };
    Ok((metrics, procs))
}

// Demo mode implementation
async fn run_demo_mode(_tls_ca: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let port = 3231;
    let url = format!("ws://127.0.0.1:{port}/ws");
    let child = spawn_demo_agent(port)?;
    let mut app = App::new();
    tokio::select! { res=app.run(&url,None)=>{ drop(child); res } _=tokio::signal::ctrl_c()=>{ drop(child); Ok(()) } }
}
struct DemoGuard {
    port: u16,
    child: std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>,
}
impl Drop for DemoGuard {
    fn drop(&mut self) {
        if let Some(mut ch) = self.child.lock().unwrap().take() {
            let _ = ch.kill();
        }
        eprintln!("Stopped demo agent on port {}", self.port);
    }
}
fn spawn_demo_agent(port: u16) -> Result<DemoGuard, Box<dyn std::error::Error>> {
    let candidate = find_agent_executable();
    let mut cmd = std::process::Command::new(candidate);
    cmd.arg("--port").arg(port.to_string());
    cmd.env("SOCKTOP_ENABLE_SSL", "0");

    //JW: do not disable GPU and TEMP in demo mode
    //cmd.env("SOCKTOP_AGENT_GPU", "0");
    //cmd.env("SOCKTOP_AGENT_TEMP", "0");

    let child = cmd.spawn()?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    Ok(DemoGuard {
        port,
        child: std::sync::Arc::new(std::sync::Mutex::new(Some(child))),
    })
}
fn find_agent_executable() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            #[cfg(windows)]
            let name = "socktop_agent.exe";
            #[cfg(not(windows))]
            let name = "socktop_agent";
            let candidate = parent.join(name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from("socktop_agent")
}
