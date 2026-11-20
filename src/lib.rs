mod adopt;
pub mod cli;
pub mod model;
mod tray;
use cli::{Executor, ProcessExecutor};
use model::{text, COMMANDS};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tezbar_extension_sdk::{self as sdk, BoxFuture, CoreResult, Extension, ExtensionContext};

#[derive(Clone, Debug)]
enum Intent {
    Refresh,
    Back,
    More,
    Previous,
    Detail(Value),
    Copy(String),
    Run(Vec<String>, bool),
    Package(String, Value),
    AddTap,
    Menu,
    Select(Value),
    SelectAll,
    UpgradeSelected,
}
type Actions = Arc<Mutex<HashMap<String, HashMap<String, Intent>>>>;
fn matching_rows<'a>(command: &str, state: &'a Value) -> Vec<&'a Value> {
    let query = text(state, "query").to_lowercase();
    state["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            command == "search"
                || format!("{} {}", text(row, "name"), text(row, "description"))
                    .to_lowercase()
                    .contains(&query)
        })
        .collect()
}
pub struct Adapter {
    pub(crate) executor: Arc<dyn Executor>,
    actions: Actions,
    pub(crate) menu: Arc<tray::Menu>,
    pub(crate) root: Mutex<Option<ExtensionContext>>,
    mutation: tokio::sync::Mutex<()>,
}
impl Default for Adapter {
    fn default() -> Self {
        Self::with_executor(Arc::new(ProcessExecutor))
    }
}
impl Adapter {
    pub fn with_executor(executor: Arc<dyn Executor>) -> Self {
        Self {
            executor,
            actions: Arc::default(),
            menu: Arc::default(),
            root: Mutex::new(None),
            mutation: tokio::sync::Mutex::new(()),
        }
    }
    fn platform(&self, ctx: &ExtensionContext) -> Result<(), String> {
        if !["macOS", "aarch64-apple-darwin", "x86_64-apple-darwin"]
            .contains(&ctx.platform.as_str())
        {
            Err("Brew is a macOS-only extension. This Windows executable is for development verification, not Windows Homebrew support.".into())
        } else {
            Ok(())
        }
    }
    async fn run(
        &self,
        ctx: &ExtensionContext,
        prefs: &Value,
        args: Vec<String>,
        mutation: bool,
    ) -> Result<cli::Output, String> {
        if prefs["verboseLogging"] == true {
            eprintln!(
                "Brew operation: {} ({} arguments; values omitted)",
                if mutation { "write" } else { "read" },
                args.len()
            );
        }
        self.executor.run(ctx, prefs, &args, mutation).await
    }
    async fn load(
        &self,
        ctx: &ExtensionContext,
        command: &str,
        prefs: &Value,
        query: &str,
    ) -> Result<Vec<Value>, String> {
        let words = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
        match command {
            "installed" => model::packages(
                self.run(
                    ctx,
                    prefs,
                    words(&["info", "--json=v2", "--installed"]),
                    false,
                )
                .await?
                .json(false)?,
                false,
            ),
            "outdated" | "upgrade" => {
                let mut args = words(&["outdated", "--json=v2"]);
                if prefs["greedyUpgrades"] == true {
                    args.push("--greedy".into())
                }
                model::packages(self.run(ctx, prefs, args, false).await?.json(false)?, true)
            }
            "manage-services" | "services-menu" => model::services(
                self.run(ctx, prefs, words(&["services", "list", "--json"]), false)
                    .await?
                    .json(false)?,
            ),
            "manage-taps" => model::taps(
                self.run(
                    ctx,
                    prefs,
                    words(&["tap-info", "--json=v1", "--installed"]),
                    false,
                )
                .await?
                .json(false)?,
            ),
            "run-doctor" => model::findings(
                self.run(ctx, prefs, words(&["doctor", "--json"]), false)
                    .await?
                    .json(true)?,
                true,
            ),
            "show-vulnerabilities" => model::findings(
                self.run(ctx, prefs, words(&["vulns", "--json"]), false)
                    .await?
                    .json(true)?,
                false,
            ),
            "adopt-apps" => {
                let v = self
                    .run(
                        ctx,
                        prefs,
                        words(&["info", "--json=v2", "--cask", "--eval-all"]),
                        false,
                    )
                    .await?
                    .json(false)?;
                let casks = v["casks"].as_array().ok_or("Missing casks catalogue")?;
                let mut roots = vec![std::path::PathBuf::from("/Applications")];
                if let Some(home) = std::env::var_os("HOME") {
                    roots.push(std::path::PathBuf::from(home).join("Applications"))
                }
                adopt::candidates(
                    casks,
                    &roots.iter().map(|v| v.as_path()).collect::<Vec<_>>(),
                )
            }
            "search" => {
                if query.trim().is_empty() {
                    return Ok(vec![]);
                }
                if query.len() > 256 || query.starts_with('-') || query.contains(['\n', '\r', '\0'])
                {
                    return Err(
                        "Enter a search query of at most 256 characters, without a leading dash"
                            .into(),
                    );
                }
                let mut rows = Vec::new();
                for kind in ["formula", "cask"] {
                    let result = self
                        .run(
                            ctx,
                            prefs,
                            vec!["search".into(), format!("--{kind}"), query.into()],
                            false,
                        )
                        .await?
                        .require_success()?;
                    for name in result
                        .stdout
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("==>"))
                        .flat_map(str::split_whitespace)
                        .filter(|s| model::safe_name(s))
                        .take(100)
                    {
                        rows.push(json!({"name":name,"kind":kind,"description":kind,"data":{"name":name}}));
                    }
                }
                Ok(rows)
            }
            "clean-up" | "clear-cache" => Ok(vec![]),
            _ => Err("Unknown Brew command".into()),
        }
    }
    async fn confirm(&self, ctx: &ExtensionContext, args: &[String]) -> Result<bool, String> {
        let v=ctx.host("host.dialog.confirm",json!({"title":"Run Homebrew operation?","message":format!("brew {}\nHomebrew may change installed applications, files, services, or trusted code.",args.join(" ")),"buttons":["Cancel","Run"],"defaultId":0,"cancelId":0})).await?;
        if v == true || v["confirmed"] == true || v["response"] == 1 {
            Ok(true)
        } else if v == false || v["confirmed"] == false || v["response"] == 0 {
            Ok(false)
        } else {
            Err("Unexpected confirmation dialog reply".into())
        }
    }
    async fn trust_preflight(
        &self,
        ctx: &ExtensionContext,
        prefs: &Value,
        args: &[String],
    ) -> Result<bool, String> {
        if ["install", "reinstall", "upgrade"]
            .contains(&args.first().map(String::as_str).unwrap_or(""))
        {
            if let Some(name) = args.last().filter(|name| name.split('/').count() == 3) {
                let tap = name.split('/').take(2).collect::<Vec<_>>().join("/");
                let taps = self
                    .run(
                        ctx,
                        prefs,
                        vec!["tap-info".into(), "--json=v1".into(), tap.clone()],
                        false,
                    )
                    .await?
                    .json(false)?;
                let tap_info = taps
                    .as_array()
                    .ok_or("Invalid tap-info response")?
                    .iter()
                    .find(|v| v["name"].as_str() == Some(&tap))
                    .ok_or("Tapped package owner not found; refresh before installing")?;
                if tap_info["trusted"] == false {
                    let trust = self
                        .run(ctx, prefs, vec!["trust".into(), "--json=v1".into()], false)
                        .await?
                        .json(false)?;
                    let key = if args.iter().any(|a| a == "--cask") {
                        "casks"
                    } else {
                        "formulae"
                    };
                    let names = trust[key]
                        .as_array()
                        .ok_or("Invalid Homebrew trust response")?;
                    let trusted = names
                        .iter()
                        .any(|v| v.as_str().is_some_and(|v| v.eq_ignore_ascii_case(name)));
                    if !trusted {
                        let command = vec![
                            "trust".into(),
                            if key == "casks" {
                                "--cask"
                            } else {
                                "--formula"
                            }
                            .into(),
                            name.clone(),
                        ];
                        if !self.confirm(ctx, &command).await? {
                            return Ok(false);
                        }
                        self.run(ctx, prefs, command, true)
                            .await?
                            .require_success()?;
                    }
                }
            }
        }
        Ok(true)
    }
    async fn mutate(
        &self,
        ctx: &ExtensionContext,
        prefs: &Value,
        args: Vec<String>,
        terminal_pref: bool,
    ) -> Result<Option<cli::Output>, String> {
        self.mutate_checked(ctx, prefs, args, terminal_pref, None)
            .await
    }
    async fn mutate_checked(
        &self,
        ctx: &ExtensionContext,
        prefs: &Value,
        args: Vec<String>,
        terminal_pref: bool,
        adoption: Option<&Value>,
    ) -> Result<Option<cli::Output>, String> {
        // Never silently ignore a requested terminal or type into a user's shell.
        if terminal_pref && prefs["runInTerminalByDefault"] == true {
            return Err(format!("Run in {} is not implemented by this development port. Disable Run in Terminal by Default to run inside Tezbar.",prefs["terminalApp"].as_str().unwrap_or("Terminal")));
        }
        if !self.confirm(ctx, &args).await? {
            return Ok(None);
        }
        let _lane = self.mutation.lock().await;
        if !self.trust_preflight(ctx, prefs, &args).await? {
            return Ok(None);
        }
        if let Some(row) = adoption {
            adopt::revalidate(row)?;
        }
        let output = self.run(ctx, prefs, args, true).await?.require_success()?;
        if prefs["closeAfterAction"] == true {
            ctx.host("host.window.visibility", json!({"visible":false}))
                .await?;
        }
        Ok(Some(output))
    }
    fn action(
        actions: &mut HashMap<String, Intent>,
        out: &mut Vec<Value>,
        title: &str,
        intent: Intent,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        actions.insert(id.clone(), intent);
        out.push(json!({"id":id,"kind":"action","title":title}));
        id
    }
    fn render(&self, command: &str, state: &mut Value) -> Value {
        let mut map = HashMap::new();
        let mut actions = Vec::new();
        let mut children = Vec::new();
        let mut global = Vec::new();
        global.push(Self::action(
            &mut map,
            &mut actions,
            "Refresh",
            Intent::Refresh,
        ));
        if command == "manage-taps" {
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Add Tap from Search Text",
                Intent::AddTap,
            ));
        }
        if command == "services-menu" {
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Enable Services Menu",
                Intent::Menu,
            ));
        }
        if ["outdated", "upgrade"].contains(&command) {
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Update Homebrew Index",
                Intent::Run(vec!["update".into()], true),
            ));
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Upgrade Selected Packages",
                Intent::UpgradeSelected,
            ));
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Select All Displayed Upgrades",
                Intent::SelectAll,
            ));
        }
        if command == "clean-up" {
            let mut args = vec!["cleanup".into()];
            if state["preferences"]["withoutThreshold"] == true {
                args.push("--prune=all".into())
            }
            global.push(Self::action(
                &mut map,
                &mut actions,
                "Clean Homebrew Cache",
                Intent::Run(args, true),
            ));
        }
        if let Some(detail) = state["detail"].as_str() {
            let back = Self::action(&mut map, &mut actions, "Back", Intent::Back);
            let copy = Self::action(
                &mut map,
                &mut actions,
                "Copy Report",
                Intent::Copy(detail.into()),
            );
            let root = json!({"type":"Detail","props":{"markdown":detail,"actionIds":[back,copy]}});
            self.actions
                .lock()
                .unwrap()
                .insert(text(state, "owner"), map);
            return sdk::view("Brew", root, actions);
        }
        let prefs = state["preferences"].clone();
        let offset = state["offset"].as_u64().unwrap_or(0) as usize;
        let limit = 100;
        let matching = matching_rows(command, state);
        for row in matching.iter().skip(offset).take(limit) {
            let mut ids = Vec::new();
            ids.push(Self::action(
                &mut map,
                &mut actions,
                "Show Details",
                Intent::Detail((*row).clone()),
            ));
            let kind = text(row, "kind");
            let verbs: &[&str] = match kind.as_str() {
                "service" => &["start", "stop", "restart"],
                "tap" => &["trust", "untrust", "untap"],
                "formula" | "cask" if command == "adopt-apps" => &["adopt"],
                "formula" | "cask" if command == "search" => &["install"],
                "formula" | "cask" if ["outdated", "upgrade"].contains(&command) => {
                    &["upgrade", "pin", "unpin"]
                }
                "formula" | "cask" => &["uninstall", "reinstall", "pin", "unpin"],
                _ => &[],
            };
            for verb in verbs {
                ids.push(Self::action(
                    &mut map,
                    &mut actions,
                    &format!("{} {}", verb, text(row, "name")),
                    Intent::Package((*verb).into(), (*row).clone()),
                ));
            }
            if ["outdated", "upgrade"].contains(&command) {
                ids.push(Self::action(
                    &mut map,
                    &mut actions,
                    "Toggle Selection",
                    Intent::Select((*row).clone()),
                ));
            }
            ids.extend(global.clone());
            let mut props = json!({"id":format!("{}:{}",kind,text(row,"name")),"title":row["name"],"subtitle":row["description"],"actionIds":ids});
            if command == "search" && prefs["showMetadataPanel"] != false {
                props["detail"] = json!({"type":"List.Item.Detail","props":{"markdown":format!("{}\n\nUse Show Details to load the current Homebrew package metadata.",text(row,"name"))}})
            }
            if row["pinned"] == true {
                props["accessories"] = json!([{"text":"Pinned"}]);
            }
            children.push(json!({"type":"List.Item","props":props}));
        }
        if offset > 0 {
            let previous = Self::action(&mut map, &mut actions, "Previous Page", Intent::Previous);
            children.push(json!({"type":"List.Item","props":{"id":"previous","title":"Previous page","actionIds":[previous]}}));
        }
        if matching.len() > offset + limit {
            let more = Self::action(&mut map, &mut actions, "Load More", Intent::More);
            children.push(json!({"type":"List.Item","props":{"id":"more","title":format!("Next page ({} remaining)",matching.len()-offset-limit),"actionIds":[more]}}));
        }
        if children.is_empty() {
            children.push(json!({"type":"List.EmptyView","props":{"title":if command=="search"{"Search Homebrew"}else{"No results"},"description":state["notice"].as_str().unwrap_or("Type to search, or choose Refresh."),"actionIds":global}}));
        }
        let root = json!({"type":"List","props":{"navigationTitle":format!("Brew — {}",state["notice"].as_str().unwrap_or(command)),"searchText":state["query"],"searchBarPlaceholder":if command=="manage-taps"{"Filter or enter user/repo to add"}else{"Search packages"},"__hasServerSearch":true,"isShowingDetail":command=="search"&&prefs["showMetadataPanel"]!=false},"children":children});
        self.actions
            .lock()
            .unwrap()
            .insert(text(state, "owner"), map);
        sdk::view("Brew", root, actions)
    }
    async fn refresh(&self, ctx: &ExtensionContext, command: &str, state: &mut Value) {
        match self
            .load(ctx, command, &state["preferences"], &text(state, "query"))
            .await
        {
            Ok(mut rows) => {
                if state["preferences"]["pinnedFirst"] != false {
                    rows.sort_by_key(|v| v["pinned"] != true)
                }
                state["rows"] = json!(rows);
                state["notice"] = json!(if ["outdated", "upgrade"].contains(&command) {
                    "Index not auto-updated. Use Update Homebrew Index for current upgrades."
                } else {
                    ""
                });
            }
            Err(error) => {
                state["rows"] = json!([]);
                state["notice"] = json!(error);
            }
        }
    }
}

impl Extension for Adapter {
    fn id(&self) -> &'static str {
        "com.tezbar.native.brew"
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn commands(&self) -> &'static [&'static str] {
        COMMANDS
    }
    fn initialize<'a>(&'a self, ctx: &'a ExtensionContext) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            *self.root.lock().unwrap() = Some(ctx.clone());
            Ok(json!({"ready":true}))
        })
    }
    fn handle<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        command: &'a str,
        operation: &'a str,
        payload: &'a Value,
        state: &'a mut Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            if operation == "extension:dispose-session" {
                self.actions.lock().unwrap().remove(&text(state, "owner"));
                return Ok(json!({}));
            }
            self.platform(ctx)?;
            if !COMMANDS.contains(&command) {
                return Err("Unknown command".into());
            }
            if operation == "extension:run-command" {
                *state = json!({"owner":uuid::Uuid::new_v4().to_string(),"preferences":payload["preferences"].clone(),"rows":[],"query":payload["argumentValues"][if command=="manage-taps"{"tap"}else{"search"}].as_str().unwrap_or(""),"limit":100,"selection":[]});
                if command == "clear-cache" {
                    self.actions.lock().unwrap().clear();
                    return Ok(sdk::no_view(
                        "Extension view caches cleared. Homebrew's own cache was not removed.",
                    ));
                }
                if command == "clean-up" {
                    let mut args = vec!["cleanup".into()];
                    if state["preferences"]["withoutThreshold"] == true {
                        args.push("--prune=all".into())
                    }
                    let result = self.mutate(ctx, &state["preferences"], args, false).await?;
                    return Ok(sdk::no_view(if result.is_some() {
                        "Homebrew cleanup completed"
                    } else {
                        "Cleanup cancelled"
                    }));
                }
                self.refresh(ctx, command, state).await;
            } else if operation == "extension:search-text-changed" {
                let query = payload["text"]
                    .as_str()
                    .or_else(|| payload["searchText"].as_str())
                    .unwrap_or("");
                if query.len() > 256 {
                    return Err("Search text exceeds 256 characters".into());
                }
                state["query"] = json!(query);
                state["offset"] = json!(0);
                if command == "search" {
                    self.refresh(ctx, command, state).await;
                }
            } else if operation == "extension:refresh-session" {
                self.refresh(ctx, command, state).await;
            } else if operation == "extension:invoke-action" {
                let owner = text(state, "owner");
                let intent = self
                    .actions
                    .lock()
                    .unwrap()
                    .get_mut(&owner)
                    .and_then(|map| map.remove(payload["actionId"].as_str().unwrap_or("")))
                    .ok_or("Stale action. Refresh before trying again.")?;
                let result: Result<(), String> = async {
                    match intent {
                        Intent::Back => state["detail"] = Value::Null,
                        Intent::More => {
                            state["offset"] = json!(state["offset"].as_u64().unwrap_or(0) + 100)
                        }
                        Intent::Previous => {
                            state["offset"] =
                                json!(state["offset"].as_u64().unwrap_or(0).saturating_sub(100))
                        }
                        Intent::Refresh => {
                            state["detail"] = Value::Null;
                            self.refresh(ctx, command, state).await;
                        }
                        Intent::Copy(value) => {
                            let reply = sdk::copy(ctx, &value).await?;
                            if reply["ok"] != true {
                                return Err("Clipboard write was not acknowledged".into());
                            }
                        }
                        Intent::Detail(row) => {
                            let value = if ["cask", "formula"]
                                .contains(&text(&row, "kind").as_str())
                                && command != "adopt-apps"
                            {
                                self.run(
                                    ctx,
                                    &state["preferences"],
                                    vec![
                                        "info".into(),
                                        "--json=v2".into(),
                                        format!("--{}", text(&row, "kind")),
                                        model::checked_name(&row["name"])?,
                                    ],
                                    false,
                                )
                                .await?
                                .json(false)?
                            } else {
                                row["data"].clone()
                            };
                            state["detail"] = json!(format!(
                                "```json\n{}\n```",
                                serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
                            ));
                        }
                        Intent::Run(args, write) => {
                            if write {
                                self.mutate(ctx, &state["preferences"], args, false).await?;
                            }
                            self.refresh(ctx, command, state).await;
                        }
                        Intent::Package(verb, row) => {
                            if verb == "adopt" {
                                adopt::revalidate(&row)?;
                            }
                            let args = model::mutation_args(&verb, &row, &state["preferences"])?;
                            if let Some(output) = self
                                .mutate_checked(
                                    ctx,
                                    &state["preferences"],
                                    args,
                                    ["install", "uninstall", "reinstall"].contains(&verb.as_str()),
                                    if verb == "adopt" { Some(&row) } else { None },
                                )
                                .await?
                            {
                                state["detail"] = json!(format!(
                                    "```text\n{}\n{}\n```",
                                    output.stdout, output.stderr
                                ));
                                self.refresh(ctx, command, state).await;
                            } else {
                                state["notice"] = json!("Operation cancelled");
                            }
                        }
                        Intent::AddTap => {
                            let name = model::checked_name(&state["query"])?;
                            if name.split('/').count() != 2 {
                                return Err("Enter tap as user/repo".into());
                            }
                            self.mutate(
                                ctx,
                                &state["preferences"],
                                vec!["tap".into(), name],
                                false,
                            )
                            .await?;
                            self.refresh(ctx, command, state).await;
                        }
                        Intent::SelectAll => {
                            let offset = state["offset"].as_u64().unwrap_or(0) as usize;
                            let selected: Vec<_> = matching_rows(command, state)
                                .into_iter()
                                .skip(offset)
                                .take(100)
                                .cloned()
                                .collect();
                            state["selection"] = json!(selected);
                            state["notice"] = json!(format!(
                                "{} packages selected",
                                state["selection"].as_array().map_or(0, Vec::len)
                            ));
                        }
                        Intent::Select(row) => {
                            let rows = state["selection"]
                                .as_array_mut()
                                .ok_or("Invalid selection state")?;
                            if let Some(index) = rows
                                .iter()
                                .position(|v| v["name"] == row["name"] && v["kind"] == row["kind"])
                            {
                                rows.remove(index);
                            } else {
                                rows.push(row);
                            }
                            state["notice"] = json!(format!(
                                "{} packages selected",
                                state["selection"].as_array().unwrap().len()
                            ));
                        }
                        Intent::UpgradeSelected => {
                            let selected =
                                state["selection"].as_array().cloned().unwrap_or_default();
                            if selected.is_empty() {
                                return Err("Select packages with Toggle Selection first".into());
                            }
                            let args = vec!["update".into()];
                            if !self.confirm(ctx, &args).await? {
                                return Ok(());
                            }
                            let _lane = self.mutation.lock().await;
                            self.run(ctx, &state["preferences"], args, true)
                                .await?
                                .require_success()?;
                            let fresh = self
                                .load(ctx, "outdated", &state["preferences"], "")
                                .await?;
                            let mut summary = Vec::new();
                            for row in fresh.iter().filter(|row| {
                                selected
                                    .iter()
                                    .any(|v| v["name"] == row["name"] && v["kind"] == row["kind"])
                            }) {
                                if row["pinned"] == true {
                                    summary
                                        .push(format!("{}: skipped (pinned)", text(row, "name")));
                                    continue;
                                }
                                let args =
                                    model::mutation_args("upgrade", row, &state["preferences"])?;
                                if !self.confirm(ctx, &args).await? {
                                    break;
                                }
                                if !self
                                    .trust_preflight(ctx, &state["preferences"], &args)
                                    .await?
                                {
                                    summary.push(format!("{}: trust declined", text(row, "name")));
                                    continue;
                                }
                                let output =
                                    self.run(ctx, &state["preferences"], args, true).await?;
                                summary.push(format!(
                                    "{}: exit {}\n{}\n{}",
                                    text(row, "name"),
                                    output.code,
                                    output.stdout,
                                    output.stderr
                                ));
                            }
                            state["detail"] =
                                json!(format!("```text\n{}\n```", summary.join("\n")));
                            state["selection"] = json!([]);
                        }
                        Intent::Menu => {
                            self.enable_menu(ctx, state["preferences"].clone()).await?;
                            state["notice"] = json!("Services menu enabled");
                        }
                    }
                    Ok(())
                }
                .await;
                if let Err(error) = result {
                    state["notice"] = json!(error);
                }
            } else {
                return Err("Unsupported operation".into());
            }
            if let Some(detail) = state["detail"].as_str().filter(|v| v.len() > 512 * 1024) {
                let mut end = 512 * 1024;
                while !detail.is_char_boundary(end) {
                    end -= 1
                }
                state["detail"] = json!(format!(
                    "{}\n```\nReport truncated at 512 KiB.",
                    &detail[..end]
                ));
            }
            Ok(self.render(command, state))
        })
    }
    fn background_activate<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        params: &'a Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            self.platform(ctx)?;
            if params["key"] != "brew-services" {
                return Err("Unknown background key".into());
            }
            self.enable_menu(ctx, params["data"].clone()).await?;
            Ok(json!({"ready":true}))
        })
    }
    fn background_event<'a>(
        &'a self,
        ctx: &'a ExtensionContext,
        params: &'a Value,
    ) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move { self.menu_event(ctx, params).await })
    }
    fn shutdown<'a>(&'a self, _: &'a ExtensionContext) -> BoxFuture<'a, CoreResult> {
        Box::pin(async move {
            self.menu.stop();
            Ok(json!({}))
        })
    }
}
