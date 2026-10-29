# Reliability Checkpoint A - baseline, October 29, 2026

**Checkpoint A complete; runtime repairs B-F have not started.** This is a failure
inventory, not a declaration of agent readiness. No runtime/UI/Job Apps behavior was
changed. No commits, pushes, production app replacements, or live Google writes.

## Identity and reproduction

Base commit: `08f1c0cd0d7215ea10f8c6b846c67d9b95415465`.
The only working-tree changes are baseline tooling/docs and acceptance-fixture
repairs listed below. Production-source hashes matched before/after execution.

Host: Mac14,6, arm64, 32 GiB unified RAM, 12 logical CPUs; macOS 27.0.1 (26A434).
Rust 1.99.1; Swift 6.4; Ollama 0.40.13. Exact installed model:
`m2max-fast:latest`, qwen35, 9.0B, GGUF Q4_K_M, llamacpp runner. Full model metadata,
installed digest, configured context/parameters and loaded-model observations are
retained in the private evidence directory. Modelfile parameters and actual provider
request settings are not assumed equivalent: per-inference budget events are saved.

Live Core settings: nominal effort, 10 iterations, explicit 60-second model limit,
30-second command limit. Native acceptance: nominal, existing native limits. The
thinking smoke test separately exercises Off/Nominal/High with a 32,768 context and
1,024-token output allowance. It requests a call, not a complete coding task.

```sh
python3 -B tests/baseline/run.py \
  --output /tmp/lorca-baseline-NEW-RUN \
  --model m2max-fast:latest --live --native
```

Recorded run: `/Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-a-2026-10-29/`.
See `tests/baseline/README.md` for artifact contracts and limitations. Raw artifacts
stay out of Git: isolated runtime endpoint files contain local bearer tokens.

The test backend is explicitly launched from the freshly checked release binary,
whose SHA-256 is recorded, and native acceptance compiles current native sources.
The existing production endpoint does not establish a source fingerprint for its
running process; that identity remains unconfirmed rather than inferred from version
or protocol. No production application was restarted to obtain it.

## What changed (fixtures only)

- Existing live-question assertion now expects `WaitingForInput`, not obsolete
  `Incomplete`. The requirement for one real question/no side effects remains.
- Native acceptance approves `exec_command` as well as legacy `run_command`, with
  the same allowed Cargo arguments and isolated working directory.
- Long native prompt references actual `agentloop/` and tool drawer files.
- Native acceptance has explicit nominal effort (`--effort` supported), uses installed
  Ollama model names, and closes without unloading another client's model.
- Added a reusable baseline recorder, serial Core fixtures, an opt-in deterministic
  mechanics probe, and a narrow generated-slider behavior oracle. No dependencies.

The first broad-frontend attempt was contaminated by the diagnostic approval matcher
rejecting `./index.html` instead of accepting the equivalent allowed `index.html`.
It was retained, classified as fixture drift, and rerun after correcting only that
matcher. Traversal/absolute/unrelated paths remain rejected. Do not count this first
attempt as model failure. All results, including failed attempts, are retained.

## Actual checks

| Check | Result |
| --- | --- |
| `cargo test` | 175 passed, zero failed, six ignored (includes the new opt-in recorder) |
| `cargo clippy --all-targets -- -D warnings` | Exit 0 |
| `cargo fmt --check` | Exit 0 |
| Job Apps Python unittest discovery | 111 passed; mocked integration boundaries |
| Restart Python unittest discovery | Six passed |
| `cargo build --release` | Exit 0 |
| `xcrun swift build -c release` | Exit 0 |
| `xcrun swift test` | Unavailable: missing XCTest under installed CLT; exit 1 |
| Explicit mechanics diagnostic | Executed successfully; observed defects below remain |
| Existing opt-in Qwen live tests | Both passed after stale waiting-status repair |
| Thinking structured-call smoke | Off/Nominal/High passed; no tool execution |
| Repaired native acceptance | All three stages passed |
| Independent native fixture `cargo test --offline` | Exit 0 after repair |

No custom assertion shim is being counted as official XCTest. No aggregate test/model
pass percentage is claimed. The full long-context native run and live truncated-call
stress test were not run in A; existing deterministic Rust regressions ran normally.

## Failure inventory

| ID | Classification | Fresh observation and independent evidence | Later work |
| --- | --- | --- | --- |
| A-F01 | Harness defect | `Do not edit other source files` blocks the correct target patch. Rust source unchanged, independent Cargo exit 101. Positive target scope fixes it and independent Cargo exits 0. | B: task/scope |
| A-F02 | Harness defect | `Create a file.` yields reasonable plain-text clarification but ends Incomplete after repetition; no files written. Explicit structured question followed by answer creates exactly empty CHOSEN.md. | B/C: question lifecycle |
| A-F03 | Harness defect | Explicit cancellation plus `2+2` is appended as User decision to obsolete create-file objective. Incomplete; no edit approvals or files. | B: message routing |
| A-F04 | Harness defect | No-evidence probe considers `Make the gradient control change hue across the rainbow` satisfied. Exact long incident follow-up is recorded separately. Short `The slider ... fix it` DOES create an edit requirement: phrasing matters. | B: outcomes |
| A-F05 | Harness defect | Real-format command evidence with exit 101 fails run-and-report requirements; direct public verification probe. This is not a fresh live full report-only task. | B: completion |
| A-F06 | Harness/configuration defect | Scheduling an explicit numeric 120 seconds yields Off 180, Nominal 240, High 360; other timeout semantics not inferred from this probe. | D: budgets |
| A-F07 | Model/output defect | Corrected broad frontend run completes, removes stray text and passes JS syntax, but only rotates fixed color stops. Partial improvement, not requested hue sweep. | D/F: strategy/evaluation |
| A-F08 | Model/output + verification gap | Precise hue fixture completes and passes JS syntax. Sampled first-stop hues are 0,85,170 at minimum/one-third/two-thirds, not requested 0,120,240. Narrow DOM-stub oracle fails; no browser appearance claim. | B/D/F: truthful checks |
| A-F09 | Safety/outcome gap | For a >8,192-byte fixture, deletion of observed prefix prepares for approval and would leave 866 bytes. NOT executed. Path verification is not full-content knowledge or semantic preservation. | C: editing |
| A-F10 | Missing capability | Node/Python command preparation rejected, Cargo accepted; no general frontend check capability through the agent. | E: commands |
| A-F11 | Environment issue | Official Swift tests fail to import XCTest. Native release compiles and native acceptance runs; neither substitutes for XCTest. | F/environment |
| A-F12 | Fixture drift (repaired) | Old waiting status, command name, module paths/model example; diagnostic `./` matcher corrected with restricted-path checks. | A complete |
| A-F13 | Unconfirmed | Current long-context endurance, crash-effect reconciliation, arbitrary-model parity, production backend source identity, real Google integrations not established by this run. | C/D/F or separate scope |

## Preserved successes

- Exact named empty-file creation; no extra files or unnecessary Cargo/questions in
  the existing live test.
- Explicit structured question -> WaitingForInput -> user answer -> exact empty file.
- Positive-scoped Rust repair with actual approved Cargo execution and independent
  passing tests (13.39 seconds in the Core experiment).
- Native README update, Rust repair/test, contextual explanation; correct final source.
- Actual partial read/nextOffset reaches the tail beyond the first excerpt.
- Actual external modification after patch preparation rejects execution and retains
  `human edit` unchanged.
- Existing real filesystem/process/authorization/cancellation/idempotency tests pass.
  These tests provide bounded evidence; A does not certify every crash boundary.

## Live attempts retained

| Case | Application result | Independent interpretation |
| --- | --- | --- |
| Natural clarification | Incomplete, 15.71 s | Harness waiting-state defect |
| Broad frontend, initial | Denied, 17.97 s | Diagnostic matcher defect; excluded from model judgment |
| Negative-scoped Rust | Incomplete, 11.14 s | Correct edit blocked; tests fail |
| Structured question + answer | Waiting 3.94 s; Completed 6.24 s | Exact empty file, pass |
| Structured question + cancel/new task | Waiting 4.77 s; Incomplete 18.03 s | Obsolete objective retained |
| Positive-scoped Rust | Completed, 13.39 s | Independent tests pass |
| Precise frontend | Completed, 27.46 s | Structure/JS pass; hue checkpoints fail |
| Broad frontend, corrected matcher | Completed, 25.56 s | Structure/JS pass; angle-only behavior |

These are baseline attempts, not the repeated final evaluation. No repeat-until-pass
filtering was used; the only rerun corrected a recorded fixture error.

## Exit-gate assessment and next dependency

A now has reproducible fixtures, machine-readable observations, independent readbacks,
classified failures, repaired stale tests and preserved successes. Unavailable checks
are explicitly listed. No agent-runtime defect above was fixed or redefined as success.

Checkpoint B must address active task, scoped authority and outcome completion as one
coherent repair. General command execution is deliberately still restricted until
that contract and lifecycle safeguards are addressed. A is a measured starting point,
not permission to skip B-D or a claim that Lorca is already dependable.

## Follow-up: official XCTest environment resolved

After installing Xcode 27.1 (27A9275), the official suite ran with
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test`
from `macos/`: **91 tests passed, zero failures, exit 0**, 16.041 seconds
for test execution. The earlier CLT-only result is retained as historical evidence;
the XCTest availability blocker is now resolved when using this Xcode. The global
`xcode-select` setting was not changed. No runtime code or test expectations changed.
Log: `xcode-xctest.log` in the baseline evidence directory.
