//! Synthetic CLI fixture. This executable is never shipped in extension ZIPs.
use serde_json::json;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(path) = std::env::var_os("TEZBAR_BREW_FIXTURE_LOG") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f,"{}",json!({"args":args,"noAutoUpdate":std::env::var("HOMEBREW_NO_AUTO_UPDATE").ok(),"concurrency":std::env::var("HOMEBREW_DOWNLOAD_CONCURRENCY").ok()})).unwrap();
    }
    match args.first().map(String::as_str).unwrap_or("") {
        "echo" => println!(
            "{}",
            json!({"args":&args[1..],"noAutoUpdate":std::env::var("HOMEBREW_NO_AUTO_UPDATE").ok(),"concurrency":std::env::var("HOMEBREW_DOWNLOAD_CONCURRENCY").ok()})
        ),
        "overflow" => print!("{}", "x".repeat(2 * 1024 * 1024)),
        "wait" => std::thread::sleep(std::time::Duration::from_secs(60)),
        "fail" => {
            eprintln!("synthetic CLI error");
            std::process::exit(2)
        }
        "info" | "outdated" => println!(
            "{}",
            json!({"formulae":[{"name":"jq","full_name":"jq","desc":"synthetic JSON utility","installed":[{"version":"1.0"}],"versions":{"stable":"2.0"},"pinned":false}],"casks":[{"name":"Test App","token":"test-app","desc":"synthetic app","version":"1.0","installed":"1.0"}]})
        ),
        "search" => println!("jq\ntest-app"),
        "services" if args.get(1).map(String::as_str) == Some("list") => println!(
            "{}",
            json!([{"name":"postgresql@18","status":"started","user":"fixture","file":"/synthetic/service.plist","exit_code":0}])
        ),
        "tap-info" => println!(
            "{}",
            json!([{"name":"fixture/tap","installed":true,"trusted":false,"formula_names":["fixture/tap/jq"],"cask_tokens":[]} ])
        ),
        "doctor" => {
            println!(
                "{}",
                json!({"tier":1,"findings":[{"text":"Synthetic finding","tier":1,"affects":[],"links":[],"remediation":{"commands":["do-not-run-this"],"text":"Manual fixture"}}]})
            );
            std::process::exit(1)
        }
        "vulns" => {
            println!(
                "{}",
                json!({"findings":[{"formula":"jq","version":"1.0","vulnerabilities":[{"id":"CVE-TEST","severity":"HIGH","summary":"synthetic","fixed_versions":["2.0"]}]}],"skipped_formulae":["fixture/skipped"],"unscanned_formulae":{"unknown":"untrusted_tap"},"outdated_without_sbom":[]})
            );
            std::process::exit(1)
        }
        _ => println!("Synthetic operation completed; no Homebrew or system package was changed."),
    }
}
