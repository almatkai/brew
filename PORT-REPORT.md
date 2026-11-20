# Brew Rust port ? development, partial parity

Standalone source: `C:/Users/alikh/Tezbar-rust-ports/brew`. Source frozen on 2026-11-20. The original 12 command IDs and all nine extension preference definitions, both command preference definitions, arguments, disabledByDefault, resources and upstream MIT notice are retained. Manifest platform remains **macOS only**. This is a functional development implementation with explicit gaps, not full upstream or release acceptance.

Upstream source: https://github.com/raycast/extensions/tree/b1e7745b664f9cde82b6e671fc760e8488501a05/extensions/brew . Immutable published SDK revision `e955911f0bf5b558f9d3eb5183df0adc95c22af2` is pinned in Cargo.toml and Cargo.lock. Original TS source and its tests are retained under reference/ for review, outside Rust application entrypoints. Native implementation uses Rust/JSONL and no Bun, React or Node worker.

## Implemented command behavior

| Original command | Rust behavior | Differences / remaining parity |
| --- | --- | --- |
| installed | `brew info --json=v2 --installed`, separate formula/cask identities, filter, 100-row pages, details, reinstall/uninstall/pin/unpin | Pinned-first reads supplied JSON pinned flags; upstream separately reads pin directories. No live macOS acceptance. |
| search | Separate CLI formula and cask searches, explicit argument/server search, headings excluded, detail JSON fetched by exact typed identifier, install actions | CLI results replace upstream remote fuzzy metadata index and analytics; the side panel initially explains how to load details rather than automatically fetching every selected item. |
| outdated | `brew outdated --json=v2`, greedy flag, read-only opening, explicit Update Homebrew Index, selection | Source index refresh is deliberately explicit rather than automatic on open; index freshness is stated in the view. |
| show-vulnerabilities | Homebrew 7 `vulns --json`, exit 1 findings accepted, open finding objects and skipped/unscanned/outdated-SBOM coverage warnings retained | Shows raw detailed advisories rather than source severity sorting/OSV enriched views; patched advisory taxonomy is not independently accepted. Older unsupported CLI output reports errors. |
| run-doctor | `brew doctor --json`, exit 1 findings accepted, structured finding details/copy | Does not execute arbitrary shell/sudo remediation strings from findings; those remain visible in the copied report for manual review. No claim of automatic fixes. |
| adopt-apps | Bounded full cask JSON, /Applications and ~/Applications matching, exact published bundle ID and exact version, target rename handling, symlink checks, identity recheck after confirmation, `install --cask --adopt` | Conservative subset: ambiguous/identity-free/version-different candidates and casks with binary/manpage/completion artifacts are omitted. Source's broader confidence/heuristic/version/link-source matching and ignore list are not ported. 64 MiB output cap can refuse a larger catalogue. Filesystem checks retain a TOCTOU interval. |
| upgrade | Review/select one or all observed upgrades, confirm update, fresh outdated intersection, preserve formula/cask kind, skip JSON-pinned packages, explicit per-package confirmation, raw per-package exit/output summary, continue after CLI failures | No prefetch/progress parser, native pin-directory snapshot or source stale-process detector. Successful exit with skip warnings is reported as exit/output, not falsely labelled upgraded. Runtime host cancellation remains bounded by the 300-second command deadline. |
| manage-services | JSON service list with per-service start/stop/restart actions | No separate all-services actions in the list; all-service start/stop/restart are available in the services menu. |
| manage-taps | Installed tap list/details, add typed user/repo, trust/untrust using `--tap`, untap | No custom remote URL/pasted shell parsing, tap package browser, dependency-removal workflows or upstream Git preflight UI. Fully qualified install/reinstall/upgrade checks tap trust first and obtains separate package-scoped trust confirmation when needed. |
| services-menu | Durable background lease, root-owned process scope, 60-second polling, per-service and all-service controls, refresh/hide/release, restoration hook | Menu shows first 28 services plus an explicit overflow notice to respect generic host's 128-item bound. Uses text title rather than original SVG tray rendering. Real tray/restore/platform acceptance pending. |
| clean-up | Confirmed `brew cleanup`, original withoutThreshold preference adds `--prune=all` | Adds an explicit operation confirmation. Real deletion is never exercised in verification. |
| clear-cache | Clears extension-owned in-memory view/action snapshots, preserving Homebrew cache and installed packages | No persistent remote metadata cache exists in this implementation; legacy Raycast caches/settings are not read or migrated. |

## Preference and host limitations

`customBrewPath` is an exact absolute executable path, passed literally to Command without a shell. Standard /opt/homebrew/bin/brew and /usr/local/bin/brew are the fallbacks. `greedyUpgrades`, `zapCask`, `disableDownloadConcurrency`, `pinnedFirst`, `closeAfterAction`, `verboseLogging`, `showMetadataPanel`, and `withoutThreshold` affect their implemented flows. Extra diagnostics omit argument values.

**External terminal execution is not implemented.** All original terminalApp choices and runInTerminalByDefault definitions are preserved. When an install/uninstall/reinstall requests that preference, the action reports a terminal-specific error and asks the user to disable the preference for in-Tezbar execution; it never silently runs elsewhere or types into an existing terminal. There is no embedded askpass/password capture; sudo/interactive operations can fail and require a separately implemented terminal path. This is a release parity gap.

All direct subprocess work uses argument arrays, null stdin, bounded stdout/stderr, deadlines and kill-on-drop. Read-only operations set HOMEBREW_NO_AUTO_UPDATE; mutation operations retain the real environment and set concurrency preference. macOS process groups own spawned compiler/downloader descendants during cancellation. This Unix group behavior has not executed on Windows; the public platform declaration remains macOS-only. Windows tests cover the direct owned fake child and timeout, not a claim of Windows descendant containment.

Mutation actions are generated per render and consumed outside the SDK's staged session state before awaiting host effects, so a repeated/stale action cannot replay an interrupted side effect. Mutations serialize process-wide. Confirmation cancellation and declined package-scoped trust perform no mutation. Cancelled/timed-out operations report uncertainty rather than automatic retry.

## Verification

Local CARGO_BUILD_JOBS=1 format check, `cargo test --locked`, and `cargo build --locked --bin tezbar-brew` passed. **22 Rust tests** cover adoption identity/rename/recheck; literal argument injection; parser/error/exit handling; formula/cask collisions; coverage warnings; custom executable paths; confirmed/cancelled actions; stale replay; external-terminal refusal; greedy read-only listing; package-scoped trust accepted/declined; bounded pagination; owned fake child environment/failure/timeout; and all command Windows-platform gates.

The exact ZIP-extracted production extension executable also passed **22 protocol fixture checks**: all12 command entrypoints, selected install/uninstall/service/tap operations, fresh upgrade selection, stale callback refusal, services-menu durable retain/root callback after view disposal/release, and all12 actual Windows-target requests refused before CLI access. For functional fixture checks the Windows executable received an explicitly synthetic macOS target and a synthetic fake-brew.exe via customBrewPath. That is protocol/logic evidence only, not macOS functionality or Windows Homebrew support. The fake CLI was never bundled in the extension ZIP. All host effects were intercepted by the fixture peer; no real Homebrew, system package, service, clipboard, user profile, account, or terminal was changed.

No production importer acceptance, native macOS compilation, Homebrew invocation, desktop UI/tray/permission check, signing, public release or remote catalog availability is claimed. The parent owns later Mac/importer/catalog integration. No app/SDK/catalog edits, child agents, checkout changes, push/publication, or GitHub Actions build were used. App-wide pnpm gates remain the parent's integration task because this child only changes the standalone port and its new evidence directory.

Initial fixture harness runs exposed Windows CRLF framing and a test query filtering the intended fixture row; those harness issues were corrected. Final results supersede failed development runs. All acceptance counts here refer to final logs.

## Artifacts and reproduction

Windows **development/debug** ZIP: `dist/brew-x86_64-pc-windows-msvc-development.zip`; it preserves the macOS-only manifest and has no artifact URL/pin. Exact payload inventory: 75 files plus extension.json, including all three source resources and 36 target-resolved dependency license notice sets. The package helper verifies actual license files rather than substituting SPDX strings. Every ZIP entry rehashed against the manifest.

ZIP SHA256: `8c99dad0567547973a24d1ca4d57fbaa6ff039b1e3a1f55f36e1c9cbfb40df8e`

Executable SHA256: `991cfc2994b696409f7049d628576030dd16bca5817a465754722718188595b9`

Source freeze inventory digest: `798ab11863dc1ce0e9b799bfb5d7eb6299ff6afb1f0758afdcc29125a3c1ec74`

`evidence/gates.json`, `rust-tests.log`, `rust-build.log`, `protocol-results.json`, `protocol-trace.json`, `fixture-cli-calls.json`, `source-inventory.json` and `acceptance-summary.json` hold inspectable evidence. `dist/artifact.json` and the SHA256 sidecar identify the exact package. Cargo reports Rust 1.99.1 on this Windows machine; declared minimum is 1.98. Actual Mac toolchain acceptance remains pending.

Reproduce from the standalone directory with CARGO_BUILD_JOBS=1: `python scripts/verify.py`, `python scripts/package.py`, `python scripts/protocol-fixtures.py`. For a Mac, build/test natively and package with `python3 scripts/package.py --target aarch64-apple-darwin --binary target/debug/tezbar-brew`. The final protocol helper is Windows-development-specific; do not call its simulated platform evidence native Mac acceptance.
