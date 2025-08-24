//! Unit test for port parsing logic moved out of `main.rs`.

fn parse_port<I: IntoIterator<Item = String>>(args: I, default_port: u16) -> u16 {
    let mut it = args.into_iter();
    let _ = it.next(); // program name
    let mut long: Option<String> = None;
    let mut short: Option<String> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => long = it.next(),
            "-p" => short = it.next(),
            _ if a.starts_with("--port=") => {
                if let Some((_, v)) = a.split_once('=') {
                    long = Some(v.to_string());
                }
            }
            _ => {}
        }
    }
    long.or(short)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(default_port)
}

#[test]
fn port_long_short_and_assign() {
    assert_eq!(
        parse_port(vec!["agent".into(), "--port".into(), "9001".into()], 8443),
        9001
    );
    assert_eq!(
        parse_port(vec!["agent".into(), "-p".into(), "9002".into()], 8443),
        9002
    );
    assert_eq!(
        parse_port(vec!["agent".into(), "--port=9003".into()], 8443),
        9003
    );
    assert_eq!(parse_port(vec!["agent".into()], 8443), 8443);
}
