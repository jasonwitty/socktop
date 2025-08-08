//! Small UI helpers: human-readable sizes, truncation, icons.

pub fn human(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b < K { return format!("{b:.0}B"); }
    let kb = b / K;
    if kb < K { return format!("{kb:.1}KB"); }
    let mb = kb / K;
    if mb < K { return format!("{mb:.1}MB"); }
    let gb = mb / K;
    if gb < K { return format!("{gb:.1}GB"); }
    let tb = gb / K;
    format!("{tb:.2}TB")
}

pub fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    if max <= 3 { return "...".into(); }
    let keep = max - 3;
    let left = keep / 2;
    let right = keep - left;
    format!("{}...{}", &s[..left], &s[s.len()-right..])
}

pub fn disk_icon(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains(':') { "🗄️" }
    else if n.contains("nvme") { "⚡" }
    else if n.starts_with("sd") { "💽" }
    else if n.contains("overlay") { "📦" }
    else { "🖴" }
}