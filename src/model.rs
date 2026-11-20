use serde_json::{json, Value};
pub const COMMANDS: &[&str] = &[
    "installed",
    "search",
    "outdated",
    "show-vulnerabilities",
    "run-doctor",
    "adopt-apps",
    "upgrade",
    "manage-services",
    "manage-taps",
    "services-menu",
    "clean-up",
    "clear-cache",
];
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && !name.starts_with(['-', '.', '/'])
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@+._/-".contains(&b))
        && !name.contains("//")
        && !name.ends_with('/')
}
pub fn checked_name(v: &Value) -> Result<String, String> {
    let name = v.as_str().ok_or("Missing package/service/tap name")?;
    if safe_name(name) {
        Ok(name.into())
    } else {
        Err("Invalid Homebrew identifier".into())
    }
}
pub fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}
pub fn packages(v: Value, outdated: bool) -> Result<Vec<Value>, String> {
    if !v.is_object() {
        return Err("Expected Homebrew package object".into());
    }
    let mut rows = Vec::new();
    for (key, kind) in [("formulae", "formula"), ("casks", "cask")] {
        let items = v[key]
            .as_array()
            .ok_or_else(|| format!("Missing {key} array"))?;
        for item in items {
            let name = if kind == "cask" {
                item["token"].as_str().or_else(|| item["name"].as_str())
            } else {
                item["full_name"].as_str().or_else(|| item["name"].as_str())
            }
            .ok_or("Missing package name")?;
            if !safe_name(name) {
                return Err("Unsafe package name from Homebrew".into());
            }
            rows.push(json!({"name":name,"kind":kind,"description":item["desc"],"pinned":item["pinned"].as_bool().unwrap_or(false),"data":item,"outdated":outdated}));
        }
    }
    Ok(rows)
}
pub fn services(v: Value) -> Result<Vec<Value>, String> {
    let rows = v.as_array().ok_or("Expected services array")?;
    rows.iter()
        .map(|item| {
            let name = checked_name(&item["name"])?;
            if !item["status"].is_string() {
                return Err("Missing service status".into());
            }
            Ok(json!({"name":name,"kind":"service","description":item["status"],"data":item}))
        })
        .collect()
}
pub fn taps(v: Value) -> Result<Vec<Value>, String> {
    v.as_array().ok_or("Expected tap-info array")?.iter().filter(|v|v["installed"]==true).map(|item|{let name=checked_name(&item["name"])?;Ok(json!({"name":name,"kind":"tap","description":if item["trusted"]==true{"Trusted"}else{"Not trusted"},"data":item}))}).collect()
}
pub fn findings(v: Value, doctor: bool) -> Result<Vec<Value>, String> {
    let source = v["findings"].as_array().ok_or("Missing findings array")?;
    let mut rows = Vec::new();
    for (i, item) in source.iter().enumerate() {
        if doctor {
            if !item["text"].is_string() || !item["affects"].is_array() || !item["links"].is_array()
            {
                return Err("Unexpected Homebrew doctor finding".into());
            }
            rows.push(json!({"name":format!("Finding {}",i+1),"kind":"doctor","description":item["text"],"data":item}));
        } else {
            let name = checked_name(&item["formula"])?;
            if !item["vulnerabilities"].is_array() {
                return Err("Unexpected vulnerability finding".into());
            }
            if item["vulnerabilities"].as_array().unwrap().is_empty() {
                continue;
            }
            rows.push(json!({"name":name,"kind":"vulnerability","description":item["version"],"data":item}));
        }
    }
    // Coverage warnings must remain visible even when there are no findings.
    if !doctor {
        for key in [
            "skipped_formulae",
            "unscanned_formulae",
            "outdated_without_sbom",
        ] {
            if v.get(key).is_some_and(|value| {
                value.as_array().is_some_and(|a| !a.is_empty())
                    || value.as_object().is_some_and(|a| !a.is_empty())
            }) {
                rows.push(json!({"name":key,"kind":"coverage","description":"Scan coverage warning","data":v[key]}))
            }
        }
    }
    Ok(rows)
}
pub fn mutation_args(verb: &str, row: &Value, prefs: &Value) -> Result<Vec<String>, String> {
    let name = checked_name(&row["name"])?;
    let kind = row["kind"].as_str().ok_or("Missing kind")?;
    let mut args = Vec::<String>::new();
    match (kind, verb) {
        ("service", "start" | "stop" | "restart") => {
            args.extend(["services".into(), verb.into(), name])
        }
        ("tap", "untap") => args.extend(["untap".into(), name]),
        ("tap", "trust" | "untrust") => args.extend([verb.into(), "--tap".into(), name]),
        (
            "formula" | "cask",
            "install" | "reinstall" | "uninstall" | "upgrade" | "pin" | "unpin" | "adopt",
        ) => {
            args.push(if verb == "adopt" { "install" } else { verb }.into());
            args.push(
                if kind == "cask" {
                    "--cask"
                } else {
                    "--formula"
                }
                .into(),
            );
            if verb == "uninstall" && kind == "cask" && prefs["zapCask"] == true {
                args.push("--zap".into())
            }
            if verb == "adopt" {
                args.push("--adopt".into())
            }
            args.push(name);
        }
        _ => return Err("Unsupported package action".into()),
    }
    Ok(args)
}
