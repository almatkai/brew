//! Conservative adoption: exact published bundle identity AND exact version.
//! Unlike upstream's wider heuristic, ambiguous matches never get an action.
use serde_json::{json, Value};
use std::{
    io::{Cursor, Read},
    path::Path,
};

pub fn identity(cask: &Value, bundle_id: &str, version: &str) -> bool {
    if bundle_id.is_empty() || version.is_empty() || cask["version"].as_str() != Some(version) {
        return false;
    }
    let mut ids = Vec::<String>::new();
    if let Some(artifacts) = cask["artifacts"].as_array() {
        for artifact in artifacts {
            for key in ["uninstall", "zap"] {
                let value = &artifact[key];
                let entries = if let Some(a) = value.as_array() {
                    a.clone()
                } else {
                    vec![value.clone()]
                };
                for entry in entries {
                    if let Some(id) = entry["quit"].as_str() {
                        ids.push(id.into())
                    }
                    if let Some(a) = entry["quit"].as_array() {
                        ids.extend(a.iter().filter_map(Value::as_str).map(str::to_owned));
                    }
                }
            }
        }
    }
    ids.iter().any(|id| id == bundle_id)
}
fn bundle_names(cask: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(artifacts) = cask["artifacts"].as_array() {
        for artifact in artifacts {
            if let Some(entries) = artifact["app"].as_array() {
                let renames: Vec<_> = entries
                    .iter()
                    .filter_map(|v| v["target"].as_str())
                    .chain(artifact["target"].as_str())
                    .collect();
                let targets: Vec<_> = if renames.is_empty() {
                    entries.iter().filter_map(Value::as_str).collect()
                } else {
                    renames
                };
                for target in targets {
                    if let Some(name) = Path::new(target)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .filter(|v| v.ends_with(".app"))
                    {
                        names.push(name.into())
                    }
                }
            }
        }
    }
    names
}
pub fn candidates(casks: &[Value], roots: &[&Path]) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    for cask in casks {
        if !cask["installed"].is_null() {
            continue;
        }
        // Upstream validates individual in-bundle link sources. This narrower
        // port declines linked/completion artifacts entirely until that richer
        // mapping has equivalent tests, instead of adopting a partial bundle.
        if cask["artifacts"].as_array().is_some_and(|artifacts| {
            artifacts.iter().any(|artifact| {
                [
                    "binary",
                    "manpage",
                    "bash_completion",
                    "zsh_completion",
                    "fish_completion",
                    "pwsh_completion",
                ]
                .iter()
                .any(|key| artifact.get(*key).is_some())
            })
        }) {
            continue;
        }
        for name in bundle_names(cask) {
            for root in roots {
                let path = root.join(&name);
                let info = path.join("Contents/Info.plist");
                if !info.is_file() {
                    continue;
                }
                if info.ancestors().any(|ancestor| {
                    ancestor
                        .symlink_metadata()
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                }) {
                    continue;
                }
                // A symlink bundle/Info.plist can move during scan. Never follow it
                // or claim the first scan authorizes a later install.
                if path
                    .symlink_metadata()
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
                    || info
                        .symlink_metadata()
                        .map_err(|e| e.to_string())?
                        .file_type()
                        .is_symlink()
                {
                    continue;
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&info)
                    .map_err(|e| e.to_string())?
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 1024 * 1024 {
                    return Err("App Info.plist is too large".into());
                }
                let value = plist::Value::from_reader(Cursor::new(bytes))
                    .map_err(|e| format!("Invalid app Info.plist: {e}"))?;
                let Some(dict) = value.as_dictionary() else {
                    continue;
                };
                let id = dict
                    .get("CFBundleIdentifier")
                    .and_then(plist::Value::as_string)
                    .unwrap_or("");
                let version = dict
                    .get("CFBundleShortVersionString")
                    .and_then(plist::Value::as_string)
                    .unwrap_or("");
                if !identity(cask, id, version) {
                    continue;
                }
                if let Some(token) = cask["token"]
                    .as_str()
                    .filter(|v| crate::model::safe_name(v))
                {
                    rows.push(json!({"name":token,"kind":"cask","description":format!("{} — {}",name,version),"data":cask,"adoptPath":path,"bundleId":id,"bundleVersion":version}));
                }
            }
        }
    }
    Ok(rows)
}
pub fn revalidate(row: &Value) -> Result<(), String> {
    let path = row["adoptPath"].as_str().ok_or("Missing adoption path")?;
    let path = Path::new(path);
    let root = path.parent().ok_or("Invalid adoption root")?;
    let matches = candidates(&[row["data"].clone()], &[root])?;
    if !matches.iter().any(|v| {
        v["adoptPath"] == row["adoptPath"]
            && v["bundleId"] == row["bundleId"]
            && v["bundleVersion"] == row["bundleVersion"]
    }) {
        return Err("App identity/version changed; rescan before adopting".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cask() -> Value {
        json!({"token":"sample","version":"1.0","installed":null,"artifacts":[{"app":["Archive.app",{"target":"Renamed.app"}]},{"zap":{"quit":"com.fixture.sample"}}]})
    }
    #[test]
    fn name_or_version_alone_never_authorizes_adoption() {
        let c = cask();
        assert!(!identity(&c, "com.other.app", "1.0"));
        assert!(!identity(&c, "com.fixture.sample", "2.0"));
        assert!(!identity(&c, "", "1.0"));
        assert!(identity(&c, "com.fixture.sample", "1.0"));
    }
    #[test]
    fn renamed_artifact_matches_destination_only() {
        assert_eq!(bundle_names(&cask()), ["Renamed.app"]);
    }
    #[test]
    fn bundle_identity_rechecked_before_adoption() {
        let root = tempfile::tempdir().unwrap();
        // macOS exposes its temporary directory through /var -> /private/var.
        // The fixture's approved root must be canonical so the intentionally
        // strict ancestor-link check exercises bundle identity rather than the
        // OS's unrelated temporary-directory alias.
        let root_path = root.path().canonicalize().unwrap();
        let info = root_path.join("Renamed.app/Contents/Info.plist");
        std::fs::create_dir_all(info.parent().unwrap()).unwrap();
        let write = |version: &str| {
            let dict = plist::Dictionary::from_iter([
                (
                    "CFBundleIdentifier",
                    plist::Value::String("com.fixture.sample".into()),
                ),
                (
                    "CFBundleShortVersionString",
                    plist::Value::String(version.into()),
                ),
            ]);
            plist::Value::Dictionary(dict).to_file_xml(&info).unwrap();
        };
        write("1.0");
        let rows = candidates(&[cask()], &[root_path.as_path()]).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(revalidate(&rows[0]).is_ok());
        write("2.0");
        assert!(revalidate(&rows[0]).is_err());
        #[cfg(unix)]
        {
            write("1.0");
            std::fs::rename(
                root_path.join("Renamed.app"),
                root_path.join("Original.app"),
            )
            .unwrap();
            std::os::unix::fs::symlink(
                root_path.join("Original.app"),
                root_path.join("Renamed.app"),
            )
            .unwrap();
            assert!(candidates(&[cask()], &[root_path.as_path()])
                .unwrap()
                .is_empty());
        }
    }
}
