//! `certassist`: makes certificates and signing requests, shows a certificate, and judges one against trusted anchors.
//! It is the command-line face of the SDK's Certificate Assistant (`bifrost_app::certificate_assistant`), which runs
//! the system `openssl`; no key leaves the machine and an existing key or certificate file is never overwritten.
//!
//! ```text
//! certassist create --cn NAME [--org ORG] [--country IS] [--email ADDR] [--dns HOST]... [--days N] [--ca]
//!                   [--key ec|rsa2048|rsa4096] [--request] [--out DIR]
//! certassist show FILE
//! certassist verify FILE [--anchor FILE]... [--host NAME]
//! ```
//! `create` writes `<name>.cert.pem` (or `<name>.csr.pem` with `--request`) and `<name>.key.pem` into DIR (the current
//! directory by default). `verify` exits 0 when the certificate is trusted and 1 when it is refused; with no `--anchor`
//! the certificate must be its own anchor.

use bifrost_app::certificate_assistant::{self as assistant, KeyKind, Request};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const USAGE: &str = "usage: certassist create --cn NAME [--org ORG] [--country CC] [--email ADDR] [--dns HOST]... [--days N] [--ca] [--key ec|rsa2048|rsa4096] [--request] [--out DIR]\n       certassist show FILE\n       certassist verify FILE [--anchor FILE]... [--host NAME]";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    Create { request: Request, csr: bool, out: PathBuf },
    Show { file: PathBuf },
    Verify { file: PathBuf, anchors: Vec<PathBuf>, host: Option<String> },
}

fn value<'a>(it: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<&'a String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Action, String> {
    let mut it = args.iter();
    let Some(command) = it.next() else { return Err(USAGE.to_string()) };
    match command.as_str() {
        "-h" | "--help" | "help" => Ok(Action::Help),
        "create" => {
            let mut request = Request::new("");
            let mut csr = false;
            let mut out = PathBuf::from(".");
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--cn" => request.common_name = value(&mut it, "--cn")?.trim().to_string(),
                    "--org" => request.organization = value(&mut it, "--org")?.to_string(),
                    "--country" => request.country = value(&mut it, "--country")?.to_string(),
                    "--email" => request.email = value(&mut it, "--email")?.to_string(),
                    "--dns" => request.dns_names.push(value(&mut it, "--dns")?.to_string()),
                    "--days" => {
                        let v = value(&mut it, "--days")?;
                        request.days = v.parse().map_err(|_| format!("--days wants a number, not {v}"))?;
                    }
                    "--ca" => request.is_ca = true,
                    "--key" => {
                        let v = value(&mut it, "--key")?;
                        request.key = KeyKind::parse(v).ok_or_else(|| format!("--key is ec, rsa2048 or rsa4096, not {v}"))?;
                    }
                    "--request" => csr = true,
                    "--out" => out = PathBuf::from(value(&mut it, "--out")?),
                    other => return Err(format!("unknown option {other}\n{USAGE}")),
                }
            }
            request.validate().map_err(|p| p.to_string())?;
            Ok(Action::Create { request, csr, out })
        }
        "show" => match (it.next(), it.next()) {
            (Some(file), None) => Ok(Action::Show { file: PathBuf::from(file) }),
            _ => Err(USAGE.to_string()),
        },
        "verify" => {
            let mut file = None;
            let mut anchors = Vec::new();
            let mut host = None;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--anchor" => anchors.push(PathBuf::from(value(&mut it, "--anchor")?)),
                    "--host" => host = Some(value(&mut it, "--host")?.to_string()),
                    other if other.starts_with("--") => return Err(format!("unknown option {other}\n{USAGE}")),
                    other if file.is_none() => file = Some(PathBuf::from(other)),
                    _ => return Err(USAGE.to_string()),
                }
            }
            let file = file.ok_or_else(|| USAGE.to_string())?;
            Ok(Action::Verify { file, anchors, host })
        }
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

/// A certificate as the `show` command prints it.
pub fn render_certificate(c: &bifrost_app::trust::Certificate) -> String {
    let when = |t: u64| format!("{t} (Unix time)");
    let names = if c.dns_names.is_empty() { "none".to_string() } else { c.dns_names.join(", ") };
    format!(
        "Subject: {}\nIssuer: {}\nNot before: {}\nNot after: {}\nHost names: {}\nCertificate authority: {}\nSelf-signed: {}\n",
        c.subject,
        c.issuer,
        when(c.not_before),
        when(c.not_after),
        names,
        if c.is_ca { "yes" } else { "no" },
        if c.subject == c.issuer { "yes" } else { "no" },
    )
}

/// The verdict line `verify` prints, and whether the certificate is trusted.
pub fn render_evaluation(e: &assistant::Evaluation) -> (String, bool) {
    use bifrost_app::trust::Verdict;
    match (&e.verdict, e.signature_ok) {
        (Verdict::Trusted, true) => ("trusted".to_string(), true),
        (Verdict::Trusted, false) => ("refused: no anchor's signature matches the certificate".to_string(), false),
        (Verdict::Refused(reason), _) => (format!("refused: {}", assistant::reason_text(*reason)), false),
    }
}

/// Runs the action. The text is what goes to the output; the number is the exit status.
pub fn run(action: &Action) -> Result<(String, i32), String> {
    match action {
        Action::Help => Ok((format!("{USAGE}\n"), 0)),
        Action::Create { request, csr, out } => {
            let made = if *csr { assistant::create_request(request, out) } else { assistant::create_self_signed(request, out) }.map_err(|e| e.to_string())?;
            let kind = if *csr { "request" } else { "certificate" };
            Ok((format!("{kind}: {}\nkey: {}\n", made.output.display(), made.key.display()), 0))
        }
        Action::Show { file } => {
            let c = assistant::read_certificate(file).map_err(|e| e.to_string())?;
            Ok((render_certificate(&c), 0))
        }
        Action::Verify { file, anchors, host } => {
            let anchors = if anchors.is_empty() { vec![file.clone()] } else { anchors.clone() };
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let e = assistant::evaluate_files(file, &anchors, host.as_deref(), now).map_err(|e| e.to_string())?;
            let (line, trusted) = render_evaluation(&e);
            Ok((format!("{line}\n"), if trusted { 0 } else { 1 }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::trust::{Certificate, Reason, Verdict};

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn create_reads_every_option() {
        let a = parse(&words("create --cn shop.example --org Shop --country IS --email ada@shop.example --dns shop.example --dns *.shop.example --days 90 --ca --key rsa2048 --request --out /tmp/x")).unwrap();
        let Action::Create { request, csr, out } = a else { panic!("create") };
        assert_eq!(request.common_name, "shop.example");
        assert_eq!(request.organization, "Shop");
        assert_eq!(request.country, "IS");
        assert_eq!(request.dns_names, vec!["shop.example", "*.shop.example"]);
        assert_eq!(request.days, 90);
        assert!(request.is_ca);
        assert_eq!(request.key, KeyKind::Rsa2048);
        assert!(csr);
        assert_eq!(out, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn create_defaults_to_a_year_an_ec_key_and_the_current_directory() {
        let Action::Create { request, csr, out } = parse(&words("create --cn a")).unwrap() else { panic!("create") };
        assert_eq!((request.days, request.key, request.is_ca), (365, KeyKind::EcP256, false));
        assert!(!csr);
        assert_eq!(out, PathBuf::from("."));
    }

    #[test]
    fn create_refuses_a_bad_request_before_it_runs() {
        assert_eq!(parse(&words("create")), Err("A common name is needed".to_string()));
        assert_eq!(parse(&words("create --cn a --country is")), Err("The country is two capital letters, such as IS".to_string()));
        assert!(parse(&words("create --cn a --days soon")).unwrap_err().contains("--days wants a number"));
        assert!(parse(&words("create --cn a --key dsa")).unwrap_err().contains("--key is ec"));
        assert!(parse(&words("create --cn")).unwrap_err().contains("--cn needs a value"));
        assert!(parse(&words("create --cn a --nope")).unwrap_err().contains("unknown option --nope"));
    }

    #[test]
    fn show_and_verify_read_their_files() {
        assert_eq!(parse(&words("show c.pem")), Ok(Action::Show { file: PathBuf::from("c.pem") }));
        assert!(parse(&words("show")).is_err());
        assert!(parse(&words("show a b")).is_err());
        let Action::Verify { file, anchors, host } = parse(&words("verify c.pem --anchor r.pem --anchor s.pem --host shop.example")).unwrap() else { panic!("verify") };
        assert_eq!(file, PathBuf::from("c.pem"));
        assert_eq!(anchors, vec![PathBuf::from("r.pem"), PathBuf::from("s.pem")]);
        assert_eq!(host.as_deref(), Some("shop.example"));
        assert!(parse(&words("verify")).is_err());
        assert!(parse(&words("verify a b")).is_err());
    }

    #[test]
    fn help_and_unknown_commands() {
        assert_eq!(parse(&words("--help")), Ok(Action::Help));
        assert_eq!(parse(&words("help")), Ok(Action::Help));
        assert!(parse(&words("sign x")).unwrap_err().starts_with("unknown command sign"));
        assert_eq!(parse(&[]), Err(USAGE.to_string()));
        assert_eq!(run(&Action::Help), Ok((format!("{USAGE}\n"), 0)));
    }

    fn cert() -> Certificate {
        Certificate { subject: "CN=a".into(), issuer: "CN=a".into(), dns_names: vec!["a.example".into()], not_before: 10, not_after: 20, is_ca: true }
    }

    #[test]
    fn a_certificate_is_printed_field_by_field() {
        let text = render_certificate(&cert());
        assert!(text.contains("Subject: CN=a\n") && text.contains("Host names: a.example\n"));
        assert!(text.contains("Certificate authority: yes\n") && text.contains("Self-signed: yes\n"));
        let mut other = cert();
        other.issuer = "CN=root".into();
        other.dns_names.clear();
        let text = render_certificate(&other);
        assert!(text.contains("Host names: none\n") && text.contains("Self-signed: no\n"));
    }

    #[test]
    fn the_verdict_line_says_why_a_certificate_is_refused() {
        let e = |verdict, signature_ok| assistant::Evaluation { certificate: cert(), verdict, signature_ok };
        assert_eq!(render_evaluation(&e(Verdict::Trusted, true)), ("trusted".to_string(), true));
        assert_eq!(render_evaluation(&e(Verdict::Refused(Reason::Expired), true)), ("refused: the certificate has expired".to_string(), false));
        let (line, trusted) = render_evaluation(&e(Verdict::Trusted, false));
        assert!(!trusted && line.contains("signature"));
    }

    fn openssl_present() -> bool {
        std::process::Command::new("openssl").arg("version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn create_show_and_verify_work_together_on_disk() {
        if !openssl_present() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("bifrost-certassist-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let create = parse(&words(&format!("create --cn Lab --dns lab.example --ca --days 7 --out {}", dir.display()))).unwrap();
        let (text, code) = run(&create).unwrap();
        assert_eq!(code, 0);
        assert!(text.contains("lab.cert.pem") && text.contains("lab.key.pem"), "{text}");
        let file = dir.join("lab.cert.pem");
        let (shown, _) = run(&Action::Show { file: file.clone() }).unwrap();
        assert!(shown.contains("Host names: lab.example") && shown.contains("Certificate authority: yes"), "{shown}");
        let (line, code) = run(&Action::Verify { file: file.clone(), anchors: vec![], host: Some("lab.example".into()) }).unwrap();
        assert_eq!((line.as_str(), code), ("trusted\n", 0));
        let (line, code) = run(&Action::Verify { file, anchors: vec![], host: Some("elsewhere.example".into()) }).unwrap();
        assert_eq!(code, 1);
        assert!(line.starts_with("refused: the certificate does not name this host"), "{line}");
        // running create again never overwrites the key
        assert!(run(&create).unwrap_err().contains("already exists"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
