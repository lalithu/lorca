# Native acceptance

Run on Apple Silicon with Ollama and the named model installed:

```sh
python3 macos/Acceptance/run.py --model qwen3.5:9b-q4_K_M
python3 macos/Acceptance/run.py --model m2max-fast:latest --effort nominal
python3 macos/Acceptance/run.py --long --model qwen3.5:9b-q4_K_M
```

The runner builds the existing app and compiles its real Dashboard, Mini, composer,
ChatStore and backend client into a native acceptance executable. It sends Return
through the actual chat editor, switches conversations during work, expands Mini,
and exercises local Ollama inference, streaming, approvals and persistence.
Only README.md/src/lib.rs edits and cargo test in the isolated fixture are approved.
No production conversation or workspace is used. The normal orca resource is loaded.

A fresh temporary directory holds source fixtures, native-report.json, native.log,
backend.log and an image rendered from the native view. Stage snapshots include
README/source readback. The long case copies the real Rust source and tests.
This is an automated native integration run, not a replacement for XCTest or manual
UX testing. Review the generated prose for factual accuracy; tool evidence alone
cannot prove semantic correctness. The runner exits unsuccessfully for non-completed
outcomes, missing native controls or unexpected tools in the contextual follow-up.

Use an installed Ollama model name; the alias does not imply a native MLX backend.
Thinking effort defaults to nominal and is recorded by the runner. Both exec_command
and its legacy run_command name are recognized for fixture-scoped Cargo approval.
The runner closes its session without unloading a model shared with another client.


Chunk 2 thinking and visual acceptance:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
python3 macos/Acceptance/run.py --ui --skip-build --model m2max-fast:latest --effort off \
  --sdk /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
```

Build beforehand with `LORCA_BUILD_ONLY=1 ./macos/build.sh`; never restart the
production application for acceptance. `--ui` uses isolated preferences and data,
checks live OFF/ON streams and a revision-invalidated approval, then captures real
SwiftUI components, Mini and both appearances. A separate conversation isolates
the write scenario from the explicitly read-only preceding task. Component fixtures
and captured reasoning replays are labelled; they are not live execution claims.
`ui-report.json` contains timings, states, byte counts and last-request token usage,
not reasoning text. The regular JSON report also excludes ephemeral thinking.
Screenshots can intentionally contain local-model reasoning: keep the temporary
artifacts local and remove them when no longer needed. Reopening these screenshots
is not evidence that reasoning survives a backend restart.

Final UI fixes add `--selection` (use the same model/SDK and `--skip-build` options).
This entry point uses a real NSApplication event loop, native mouse drags and Cmd+C
in actual message/reasoning/output views. The runner locates fixture text in
the native test window and exits nonzero if any selection check fails.
`selection-report.json` records copied ranges, multiline checks and the native
Copy menu. `layout-report.json` measures wrapped reasoning at
narrow/wide widths. Blue bubbles and green ON/neutral OFF/Next task labels are
captured in both appearances. These fixtures do not run a model or touch production
conversations. The runner restores original pasteboard representations afterward.

v0.5 export/advisor acceptance (isolated real backend and model):

```sh
python3 macos/Acceptance/run.py --v05 --skip-build --model m2max-fast:latest --effort off \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
```

This short read-only workflow also requests both whole-chat exports through the
native backend transport, checks original tool records, and opens/captures the
read-only model advisor. It does not automate the system Save dialog or online
Hub browsing. Existing `--selection` covers actual mouse dragging and Cmd+C.
