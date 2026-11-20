# Parent verification and preview scope

Requires Tezbar 0.3.0-beta.12 or later. The published app at the start of this work was beta.11; beta.12 app publication is a separate gate. This is a Rust development preview, not blanket upstream parity or live desktop/account acceptance.

Independent source review was completed before these repairs. Original freezes and their reports remain historical; the parent repaired source supersedes them. Builds ran locally on Windows and on an Apple Silicon Mac over Tailscale. No GitHub Actions builds or real service-account mutations were used. Source and immutable SDK provenance, original resources, Cargo.lock and dependency notices are retained.

The repaired source passed 24 Rust tests on each platform, including a deterministic delayed-refresh/Hide race and exact filtered page selection. The packaged JSONL suite has 22 fixture groups; the native Mac run uses a synthetic CLI, never real Homebrew. The adoption fixture canonicalizes the macOS temporary-directory alias and verifies symlink bundle refusal. Publication now preserves executable ZIP mode bits and keeps the source manifest portable.

The Windows executable is a protocol probe and is not Windows Homebrew support. External-terminal execution, arbitrary Doctor remediations, richer metadata and upstream adoption heuristics remain gaps; real Homebrew/GPUI acceptance is pending. See PORT-REPORT.md.
