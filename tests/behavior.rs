use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tezbar_brew::{
    cli::{self, Executor, Output},
    model, Adapter,
};
use tezbar_extension_sdk::{BoxFuture, Extension, ExtensionContext, ExtensionPaths, TestContext};
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<(Vec<String>, bool)>>,
    replies: Mutex<VecDeque<Output>>,
}
impl Executor for Fake {
    fn run<'a>(
        &'a self,
        _: &'a ExtensionContext,
        _: &'a Value,
        args: &'a [String],
        write: bool,
    ) -> BoxFuture<'a, Result<Output, String>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((args.to_vec(), write));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or("Unexpected fixture CLI invocation".into())
        })
    }
}
fn output(value: Value) -> Output {
    Output {
        code: 0,
        stdout: value.to_string(),
        stderr: String::new(),
    }
}
fn context(path: &std::path::Path) -> TestContext {
    let mut ctx = TestContext::new(
        ExtensionPaths {
            package_dir: path.into(),
            data_dir: path.into(),
            temp_dir: path.into(),
        },
        &[
            "dialog.confirm",
            "clipboard.write",
            "background.retain",
            "background.release",
            "tray.set",
            "tray.remove",
        ],
    );
    ctx.context.platform = "macOS".into();
    ctx
}
fn action(view: &Value, title: &str) -> String {
    view["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["title"] == title)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
fn packages() -> Value {
    json!({"formulae":[{"name":"jq","full_name":"jq","desc":"JSON","pinned":false}],"casks":[{"token":"sample","name":"Sample","pinned":true}]})
}

#[test]
fn injection_identifiers_rejected() {
    for value in [
        "-rf",
        "../x",
        "foo;rm",
        "x\ny",
        "foo$(date)",
        "/absolute",
        "foo//bar",
        "foo/",
    ] {
        assert!(!model::safe_name(value), "{value}");
    }
    for value in ["jq", "postgresql@18", "fixture/tap/pkg", "foo-bar", "c++"] {
        assert!(model::safe_name(value));
    }
}

#[tokio::test]
async fn select_all_displayed_respects_filter_and_page() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    let formulae: Vec<_> = (0..205)
        .map(|i| json!({"name":format!("pkg{i}")}))
        .collect();
    fake.replies
        .lock()
        .unwrap()
        .push_back(output(json!({"formulae":formulae,"casks":[]})));
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    app.handle(
        &ctx.context,
        "upgrade",
        "extension:run-command",
        &json!({}),
        &mut state,
    )
    .await
    .unwrap();
    let filtered = app
        .handle(
            &ctx.context,
            "upgrade",
            "extension:search-text-changed",
            &json!({"searchText":"pkg1"}),
            &mut state,
        )
        .await
        .unwrap();
    let page = app
        .handle(
            &ctx.context,
            "upgrade",
            "extension:invoke-action",
            &json!({"actionId":action(&filtered,"Load More")}),
            &mut state,
        )
        .await
        .unwrap();
    app.handle(
        &ctx.context,
        "upgrade",
        "extension:invoke-action",
        &json!({"actionId":action(&page,"Select All Displayed Upgrades")}),
        &mut state,
    )
    .await
    .unwrap();
    let selected = state["selection"].as_array().unwrap();
    assert_eq!(selected.len(), 11);
    assert!(selected
        .iter()
        .all(|row| row["name"].as_str().unwrap().contains("pkg1")));
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}
#[test]
fn exact_typed_mutations() {
    let prefs = json!({"zapCask":true});
    assert_eq!(
        model::mutation_args("uninstall", &json!({"name":"sample","kind":"cask"}), &prefs).unwrap(),
        ["uninstall", "--cask", "--zap", "sample"]
    );
    assert_eq!(
        model::mutation_args("restart", &json!({"name":"pg@18","kind":"service"}), &prefs).unwrap(),
        ["services", "restart", "pg@18"]
    );
    assert!(model::mutation_args("adopt", &json!({"name":"-bad","kind":"cask"}), &prefs).is_err());
}
#[test]
fn malformed_json_and_findings_exit() {
    assert!(Output {
        code: 1,
        stdout: "{".into(),
        stderr: "warning".into()
    }
    .json(true)
    .is_err());
    assert!(Output {
        code: 2,
        stdout: "{}".into(),
        stderr: "failed".into()
    }
    .json(true)
    .is_err());
    assert!(Output {
        code: 1,
        stdout: "{}".into(),
        stderr: "finding".into()
    }
    .json(true)
    .is_ok());
    assert!(model::packages(json!([]), false).is_err());
    assert!(model::services(json!([{"name":"valid"}])).is_err());
}
#[test]
fn package_type_collisions_retained() {
    let rows = model::packages(
        json!({"formulae":[{"name":"same"}],"casks":[{"token":"same"}]}),
        false,
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0]["kind"], rows[1]["kind"]);
}
#[test]
fn vulnerabilities_keep_coverage_warnings() {
    let rows=model::findings(json!({"findings":[],"skipped_formulae":["a"],"unscanned_formulae":{"b":"untrusted_tap"},"outdated_without_sbom":["c"]}),false).unwrap();
    assert_eq!(rows.len(), 3);
}
#[test]
fn doctor_rejects_unknown_shape() {
    assert!(model::findings(json!({"findings":[{"text":"bad"}]}), true).is_err());
}
#[test]
fn custom_executable_is_literal_absolute_path() {
    assert!(cli::executable(&json!({"customBrewPath":"relative/bin/brew"})).is_err());
    let p = std::env::temp_dir()
        .join("brew directory ' $ with spaces")
        .join("brew.exe");
    assert_eq!(cli::executable(&json!({"customBrewPath":p})).unwrap(), p);
}

#[tokio::test]
async fn installed_sorted_pinned_and_cancel_has_no_effect() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().push_back(output(packages()));
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({"preferences":{}}),
            &mut state,
        )
        .await
        .unwrap();
    assert_eq!(view["root"]["children"][0]["props"]["title"], "sample");
    let id = action(&view, "uninstall sample");
    ctx.reply(Ok(json!(false)));
    let v = ctx
        .run(app.handle(
            &ctx.context,
            "installed",
            "extension:invoke-action",
            &json!({"actionId":id}),
            &mut state,
        ))
        .await
        .unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
    assert!(app
        .handle(
            &ctx.context,
            "installed",
            "extension:invoke-action",
            &json!({"actionId":id}),
            &mut state
        )
        .await
        .is_err());
    assert!(!v["actions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn confirmed_uninstall_uses_exact_args() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().extend([
        output(packages()),
        output(json!({})),
        output(packages()),
    ]);
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({"preferences":{"zapCask":true}}),
            &mut state,
        )
        .await
        .unwrap();
    ctx.reply(Ok(json!({"response":1})));
    ctx.run(app.handle(
        &ctx.context,
        "installed",
        "extension:invoke-action",
        &json!({"actionId":action(&view,"uninstall sample")}),
        &mut state,
    ))
    .await
    .unwrap();
    let calls = fake.calls.lock().unwrap();
    assert_eq!(
        calls[1],
        (
            vec![
                "uninstall".into(),
                "--cask".into(),
                "--zap".into(),
                "sample".into()
            ],
            true
        )
    );
}

#[tokio::test]
async fn requested_terminal_is_not_silently_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().push_back(output(packages()));
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({"preferences":{"runInTerminalByDefault":true,"terminalApp":"iterm"}}),
            &mut state,
        )
        .await
        .unwrap();
    ctx.run(app.handle(
        &ctx.context,
        "installed",
        "extension:invoke-action",
        &json!({"actionId":action(&view,"uninstall sample")}),
        &mut state,
    ))
    .await
    .unwrap();
    assert!(state["notice"].as_str().unwrap().contains("iterm"));
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn no_read_mutation_on_outdated_open() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().push_back(output(packages()));
    let app = Adapter::with_executor(fake.clone());
    app.handle(
        &ctx.context,
        "outdated",
        "extension:run-command",
        &json!({"preferences":{"greedyUpgrades":true}}),
        &mut json!({}),
    )
    .await
    .unwrap();
    assert_eq!(
        fake.calls.lock().unwrap()[0],
        (
            vec!["outdated".into(), "--json=v2".into(), "--greedy".into()],
            false
        )
    );
}

#[tokio::test]
async fn all_command_identity_and_windows_gating() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = context(dir.path());
    ctx.context.platform = "Windows".into();
    let app = Adapter::default();
    assert_eq!(app.commands().len(), 12);
    for command in app.commands() {
        let e = app
            .handle(
                &ctx.context,
                command,
                "extension:run-command",
                &json!({}),
                &mut json!({}),
            )
            .await
            .unwrap_err();
        assert!(e.contains("macOS-only"));
    }
}

#[tokio::test]
async fn search_literal_query_and_two_types() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    for _ in 0..2 {
        fake.replies.lock().unwrap().push_back(Output {
            code: 0,
            stdout: "jq".into(),
            stderr: String::new(),
        });
    }
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    app.handle(
        &ctx.context,
        "search",
        "extension:run-command",
        &json!({"argumentValues":{"search":"$(literal) spaces"}}),
        &mut state,
    )
    .await
    .unwrap();
    assert_eq!(state["rows"].as_array().unwrap().len(), 2);
    assert_eq!(fake.calls.lock().unwrap()[0].0[2], "$(literal) spaces");
}

#[tokio::test]
async fn bounded_reader_refuses_overflow() {
    let input = std::io::Cursor::new(vec![0; 11]);
    assert!(cli::bounded_read(input, 10).await.is_err());
}

#[tokio::test]
async fn actual_child_receives_arguments_and_read_only_environment() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let args = vec!["echo".into(), "$(date); ' spaces".into()];
    let result = cli::run_process(
        &ctx.context,
        std::path::Path::new(env!("CARGO_BIN_EXE_fake-brew")),
        &[],
        &args,
        &json!({"disableDownloadConcurrency":true}),
        false,
        Duration::from_secs(10),
    )
    .await
    .unwrap()
    .json(false)
    .unwrap();
    assert_eq!(result["args"][0], args[1]);
    assert_eq!(result["noAutoUpdate"], "1");
    assert_eq!(result["concurrency"], "1");
}

#[tokio::test]
async fn actual_child_failure_and_timeout_reported() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_fake-brew"));
    let output = cli::run_process(
        &ctx.context,
        program,
        &[],
        &["fail".into()],
        &json!({}),
        true,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!(output
        .require_success()
        .unwrap_err()
        .contains("synthetic CLI error"));
    let started = std::time::Instant::now();
    assert!(cli::run_process(
        &ctx.context,
        program,
        &[],
        &["wait".into()],
        &json!({}),
        false,
        Duration::from_millis(50)
    )
    .await
    .unwrap_err()
    .contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn declined_package_trust_prevents_install_and_trust_writes() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    fake.replies.lock().unwrap().extend([
        output(json!({"formulae":[{"name":"jq","full_name":"fixture/tap/jq"}],"casks":[]})),
        output(json!([{"name":"fixture/tap","trusted":false}])),
        output(json!({"taps":[],"formulae":[],"casks":[]})),
    ]);
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({"preferences":{}}),
            &mut state,
        )
        .await
        .unwrap();
    ctx.reply(Ok(json!({"response":1})));
    ctx.reply(Ok(json!({"response":0})));
    ctx.run(app.handle(
        &ctx.context,
        "installed",
        "extension:invoke-action",
        &json!({"actionId":action(&view,"reinstall fixture/tap/jq")}),
        &mut state,
    ))
    .await
    .unwrap();
    let calls = fake.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| !call.1));
    assert_eq!(calls[1].0, ["tap-info", "--json=v1", "fixture/tap"]);
}

#[tokio::test]
async fn accepted_package_trust_is_scoped_and_precedes_install() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    let package = json!({"formulae":[{"name":"jq","full_name":"fixture/tap/jq"}],"casks":[]});
    fake.replies.lock().unwrap().extend([
        output(package.clone()),
        output(json!([{"name":"fixture/tap","trusted":false}])),
        output(json!({"taps":[],"formulae":[],"casks":[]})),
        output(json!({})),
        output(json!({})),
        output(package),
    ]);
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({"preferences":{}}),
            &mut state,
        )
        .await
        .unwrap();
    ctx.reply(Ok(json!({"response":1})));
    ctx.reply(Ok(json!({"response":1})));
    ctx.run(app.handle(
        &ctx.context,
        "installed",
        "extension:invoke-action",
        &json!({"actionId":action(&view,"reinstall fixture/tap/jq")}),
        &mut state,
    ))
    .await
    .unwrap();
    let calls = fake.calls.lock().unwrap();
    assert_eq!(
        calls[3],
        (
            vec!["trust".into(), "--formula".into(), "fixture/tap/jq".into()],
            true
        )
    );
    assert_eq!(
        calls[4],
        (
            vec![
                "reinstall".into(),
                "--formula".into(),
                "fixture/tap/jq".into()
            ],
            true
        )
    );
}

#[tokio::test]
async fn list_pagination_is_bounded_and_reversible() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = context(dir.path());
    let fake = Arc::new(Fake::default());
    let formulae: Vec<_> = (0..205)
        .map(|i| json!({"name":format!("pkg{i}")}))
        .collect();
    fake.replies
        .lock()
        .unwrap()
        .push_back(output(json!({"formulae":formulae,"casks":[]})));
    let app = Adapter::with_executor(fake.clone());
    let mut state = json!({});
    let view = app
        .handle(
            &ctx.context,
            "installed",
            "extension:run-command",
            &json!({}),
            &mut state,
        )
        .await
        .unwrap();
    assert_eq!(view["root"]["children"].as_array().unwrap().len(), 101);
    let next = app
        .handle(
            &ctx.context,
            "installed",
            "extension:invoke-action",
            &json!({"actionId":action(&view,"Load More")}),
            &mut state,
        )
        .await
        .unwrap();
    assert_eq!(next["root"]["children"][0]["props"]["title"], "pkg100");
    let back = app
        .handle(
            &ctx.context,
            "installed",
            "extension:invoke-action",
            &json!({"actionId":action(&next,"Previous Page")}),
            &mut state,
        )
        .await
        .unwrap();
    assert_eq!(back["root"]["children"][0]["props"]["title"], "pkg0");
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}
