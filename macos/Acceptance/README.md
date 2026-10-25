# Native acceptance

Run on Apple Silicon with Ollama and the named model installed:

```sh
python3 macos/Acceptance/run.py --model qwen3.5:9b-q4_K_M
python3 macos/Acceptance/run.py --model qwen3.6:27b-mlx
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
