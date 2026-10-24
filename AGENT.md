# Agent foundation (v0.4A)

- One Rust core, one active task; existing chat, Dashboard, and Mini are unchanged.
- No SwiftUI Tasks UI, MCP, subagents, or background service installation.

## Try it

```sh
cargo build
./target/debug/lorca agent --workspace "$PWD" --model qwen3.5:9b \
  --max-iterations 24 --command-timeout 120 \
  'Inspect the repository, run cargo test with my approval, and summarize the results.'
```

- Ollama must already be running and the model installed.
- `a` + Enter: Allow Once; `d`: Deny; `c` or Ctrl+C: Cancel.
- The terminal shows the complete patch or exact executable/argv/cwd before approval.
- EOF cancels. A lost client expires after 60 seconds and cancels its task.
- Agent requests reuse Rust model preparation/leases and `keep_alive: -1`.
- The testing CLI leaves the model loaded on exit; it never silently unloads another client's model.
- Use a separate workspace for editing experiments.

## Tools and boundaries

- `list_directory`: immediate entries, maximum 200.
- `read_file`: UTF-8, maximum 128 KiB; returns an 8 KiB excerpt.
- `search_files`: literal filename/content search, maximum 800 entries / 30 matches / 2 seconds; skips hidden/build folders and symlinks.
- `apply_patch`: one file per call; exact unique `old_text` replacement, or new file when `old_text` is empty. Shows the entire before/after diff. Rechecks source bytes, writes a temporary file, and atomically replaces the target. No deletions; stale files and write symlinks are rejected.
- `run_command`: explicit approval; only Cargo `test` / `check` with limited flags, `fmt --check`, or `clippy --all-targets -- -D warnings`. No shell, arbitrary executable, aliases, privileged commands, downloads, or destructive Git actions.
- `git_diff`: read-only tracked working-tree diff; disables external diff, text conversion, hooks and filesystem monitoring. Requires an internal `.git` directory; external worktree metadata is deliberately unsupported.
- File paths are relative to a capability-based workspace; traversal and escaping symlinks are denied. Git metadata is protected.
- Commands require macOS `sandbox-exec`; other platforms fail closed. Build scripts/tests inherit the sandbox. Writes are limited to the approved workspace (excluding Git metadata) and private temporary storage. Reads cover that workspace, toolchain/cache and required OS files. Environment is cleared; Cargo runs offline. Loopback networking is allowed for tests; external networking is denied.
- Timeout/cancellation kills the subprocess group, including descendants. Already completed approved actions are retained; cancellation is not rollback. Atomic file replacement is a short non-interruptible operation. Avoid concurrent editors during an approved patch; stale checks are not a filesystem-wide lock.

## Bounds and results

- Defaults: 20 model turns, 120 seconds per model request, 30 seconds per command. CLI overrides: `--max-iterations`, `--model-timeout`, `--command-timeout`.
- Maximums: 32 turns, 180 / 120 seconds; 8 calls per turn. Stops after 3 identical failures or 6 total failures.
- Tool output is bounded; commands capture 16 KiB. Model history stops at 64 KiB rather than silently dropping messages. No context compaction.
- Uses Ollama native structured calls, full assistant call messages, and correlated tool results. No parsing executable actions from prose or exposing private thinking.
- Completion requires successful tool evidence and no unresolved failed/denied command or edit. The final explanation is model generated; actual tool results and command exit codes remain the evidence.
- Task state/events are in memory: only the latest task is retained, with up to 300 events / 2 MiB. Restarting the backend discards agent tasks; saved chats/images are unaffected.

## v0.4B interface contract

Use the existing authenticated local backend (`Endpoint`, bearer token, no browser Origin):

- `POST /command`: `{"op":"agent_start","session":"…","task":"…","workspace":"/absolute/path","model":"qwen3.5:9b","limits":{"iterations":20,"modelSeconds":120,"commandSeconds":30}}` → `{"taskId":"…"}`. Model/limits are optional.
- `agent_snapshot` + `session` → latest `TaskView` or `null`.
- `GET /agent/events` + `X-Lorca-Session`: NDJSON `TaskView` snapshots, including current approval and bounded event history. Reconnect with `agent_snapshot`; deduplicate by task ID + event sequence. A sequence gap means older activity rolled off.
- `agent_decide` + `session`, `taskId`, `approvalId`, `decision` (`allow_once`, `deny`, `cancel`). Tokens are exact-action, owner-session, single-use; never send edited action arguments.
- `agent_cancel` + `session`, `taskId`. Closing/expiring the owner session cancels too.
- Status: `running`, `awaiting_approval`, `completed`, `cancelled`, `failed`.
- Events: `task_started`, `model_generating`, `tool_requested`, `approval_required`, `tool_started`, `tool_completed`, `tool_failed`, `file_changes`, `terminal_output`, `task_completed`, `task_cancelled`, `task_failed`.
- Each event has `id`, `taskId`, monotonic `sequence`, `kind`, and structured `data`. Calls have stable `callId`; approvals contain complete immutable `action` details. Terminal output identifies stdout/stderr.
- SwiftUI needs a Tasks state projection/event subscription, workspace/task input, activity/results/diff views, exact-action approval sheet, and cancel control. Keep all execution/permission logic in Rust. Pause/resume and durable task history are not implemented in v0.4A.

## Layout

- `src/agent/`: provider-independent runtime, permissions, tools, workspace access, command sandbox, event/task types.
- `src/provider/agent.rs`: native Ollama tool-call adapter.
- `src/core/agent.rs`: existing model lifecycle/session integration.
- `src/agent_cli.rs`: temporary testing interface.
- `tests/agent*.rs`: deterministic runtime, permission, filesystem and core integration tests; command sandbox tests live alongside the runner.

## Verification for this checkpoint

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and all 42 Rust tests passed.
- All 25 Swift tests and `./macos/build.sh` passed; no SwiftUI sources changed.
- Live `qwen3.5:9b` inspected this repository, requested approval for `cargo test`, executed it after Lalith's Allow Once, received exit 0 and real test output, and summarized the findings.
- Live denial prevented execution; pending-approval cancellation and foreground Ctrl+C were verified. CLI regression tests cover returning without stdin EOF and keeping the backend alive across terminal SIGINT.
- Recursive sandbox/process-inspection checks are omitted when the test suite itself runs inside an agent command sandbox; the normal full-suite run exercises them directly.
- After updating, quit existing lorca clients once and relaunch the rebuilt app/CLI so the shared backend includes the new agent API. The installed `lorca` command updates only after `cargo install --path . --locked --force`; the build above runs directly from `target/debug`.
