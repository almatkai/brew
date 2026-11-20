use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tezbar_extension_sdk::{BoxFuture, ExtensionContext};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
impl Output {
    pub fn require_success(self) -> Result<Self, String> {
        if self.code != 0 {
            Err(format!(
                "Homebrew exited {}: {}",
                self.code,
                self.stderr.chars().take(4096).collect::<String>()
            ))
        } else {
            Ok(self)
        }
    }
    pub fn json(self, findings_exit: bool) -> Result<Value, String> {
        if self.code != 0 && !(findings_exit && self.code == 1 && !self.stdout.trim().is_empty()) {
            return self.require_success().map(|_| Value::Null);
        }
        serde_json::from_str(&self.stdout).map_err(|e| format!("Invalid Homebrew JSON: {e}"))
    }
}
pub trait Executor: Send + Sync {
    fn run<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        preferences: &'a Value,
        args: &'a [String],
        mutation: bool,
    ) -> BoxFuture<'a, Result<Output, String>>;
}
#[derive(Default)]
pub struct ProcessExecutor;
pub fn executable(preferences: &Value) -> Result<PathBuf, String> {
    if let Some(path) = preferences["customBrewPath"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        if path.len() > 4096 || path.contains(['\n', '\r', '\0']) {
            return Err("Invalid custom Brew path".into());
        }
        let p = PathBuf::from(path);
        if !p.is_absolute() {
            return Err("customBrewPath must be an absolute executable path".into());
        }
        return Ok(p);
    }
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "Homebrew was not found. Set Custom Brew Executable Path in extension preferences."
                .into()
        })
}
pub async fn bounded_read(reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        Err("Homebrew output exceeded limit".into())
    } else {
        Ok(bytes)
    }
}
// A brew Ruby process can spawn compilers/downloaders; cancellation owns the
// complete POSIX process group, not just the first PID. The public port is macOS-only.
#[cfg_attr(not(unix), allow(dead_code))]
struct GroupGuard(Option<u32>);
impl Drop for GroupGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}
pub async fn run_process(
    ctx: &ExtensionContext,
    program: &Path,
    prefix: &[String],
    args: &[String],
    preferences: &Value,
    mutation: bool,
    timeout: Duration,
) -> Result<Output, String> {
    let mut command = Command::new(program);
    command
        .args(prefix)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_COLOR", "0")
        .env(
            "HOMEBREW_DOWNLOAD_CONCURRENCY",
            if preferences["disableDownloadConcurrency"].as_bool() == Some(true) {
                "1"
            } else {
                "auto"
            },
        );
    if !mutation {
        command.env("HOMEBREW_NO_AUTO_UPDATE", "1");
    }
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot launch configured Homebrew executable: {e}"))?;
    let _group = GroupGuard(child.id());
    let stdout = child.stdout.take().ok_or("Missing stdout")?;
    let stderr = child.stderr.take().ok_or("Missing stderr")?;
    let operation = async {
        let (stdout, stderr, status) = tokio::try_join!(
            bounded_read(stdout, OUTPUT_LIMIT),
            bounded_read(stderr, 1024 * 1024),
            async { child.wait().await.map_err(|e| e.to_string()) }
        )?;
        Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout: String::from_utf8(stdout).map_err(|_| "Homebrew output is not UTF-8")?,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    };
    tokio::select! {biased; _=ctx.cancelled()=>Err("Operation cancelled; package state may have changed. Refresh before retrying.".into()), result=tokio::time::timeout(timeout,operation)=>result.map_err(|_|"Homebrew timed out; package state may have changed. Refresh before retrying.".to_string())?}
}
impl Executor for ProcessExecutor {
    fn run<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        prefs: &'a Value,
        args: &'a [String],
        mutation: bool,
    ) -> BoxFuture<'a, Result<Output, String>> {
        Box::pin(async move {
            let path = executable(prefs)?;
            run_process(
                ctx,
                &path,
                &[],
                args,
                prefs,
                mutation,
                Duration::from_secs(if mutation { 1800 } else { 180 }),
            )
            .await
        })
    }
}
