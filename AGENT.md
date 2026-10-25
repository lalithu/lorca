# Agent runtime and Chats (v0.4B)

- One Rust core, one active task; agent execution is embedded in existing Chats.
- Tasks stays untouched. No MCP, subagents, or background service installation.

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

## Chats

- Choose Agent in the composer, select a workspace, and send. Chat mode preserves normal chat/images.
- Expand tool activity for arguments/results; commands and patches require exact-action inline approval.
- Scroll up to pause following; return to the bottom to resume. Mini shows basic status and opens Chats.
- Requests/final results are saved in the originating conversation; drafts, images, and IDs remain compatible.
- Only latest bounded execution activity is recoverable from the backend. The UI caches up to eight runs in memory, not on disk.
- Backend restarts mark unfinished saved requests interrupted; they are never resumed or resubmitted automatically.
- Model menu → Reload model unloads/reloads after a shared-model warning; active work must be stopped first. It reuses Rust leases and loading UI; no automatic reload.
- GUI limits: 24 model turns, 120 seconds per model request, 120 seconds per command. Broad tasks can hit context/time limits; failures remain visible and saved.

## v0.4B interface contract

Use the existing authenticated local backend (`Endpoint`, bearer token, no browser Origin):

- `POST /command`: `{"op":"agent_start","session":"…","task":"…","workspace":"/absolute/path","model":"qwen3.5:9b","limits":{"iterations":20,"modelSeconds":120,"commandSeconds":30}}` → `{"taskId":"…"}`. Model/limits are optional. Chats also sends `conversationId` and a unique `requestId`; the saved request ID prevents duplicate execution after an uncertain acknowledgement.
- `agent_snapshot` + `session` → latest `TaskView` or `null`.
- `GET /agent/events` + `X-Lorca-Session`: NDJSON `TaskView` snapshots, including current approval and bounded event history. Reconnect with `agent_snapshot`; deduplicate by task ID + event sequence. A sequence gap means older activity rolled off.
- `agent_decide` + `session`, `taskId`, `approvalId`, `decision` (`allow_once`, `deny`, `cancel`). Tokens are exact-action, owner-session, single-use; never send edited action arguments.
- `agent_cancel` + `session`, `taskId`. Closing/expiring the owner session cancels too.
- Status: `running`, `awaiting_approval`, `completed`, `cancelled`, `failed`.
- Events: `task_started`, `model_generating`, `assistant_message`, `tool_requested`, `approval_required`, `tool_started`, `tool_completed`, `tool_failed`, `file_changes`, `terminal_output`, `task_completed`, `task_cancelled`, `task_failed`.
- Each event has `id`, `taskId`, monotonic `sequence`, `kind`, and structured `data`. Calls have stable `callId`; approvals contain complete immutable `action` details. Terminal output identifies stdout/stderr.
- `TaskView` includes optional `conversationId` / `requestId`. `assistant_message` contains only public assistant content, never private thinking. SwiftUI projects these events in Chats; all execution and permission enforcement stays in Rust. Pause/resume and durable execution history are not implemented.

## Layout

- `src/agent/`: provider-independent runtime, permissions, tools, workspace access, command sandbox, event/task types.
- `src/provider/agent.rs`: native Ollama tool-call adapter.
- `src/core/agent.rs`, `agent_chats.rs`: existing model lifecycle/session integration, saved request/result association, and interruption recovery.
- `src/agent_cli.rs`: temporary testing interface.
- `tests/agent*.rs`: deterministic runtime, permission, filesystem and core integration tests; command sandbox tests live alongside the runner.

## Changed-file map

- New Swift agent modules: `AgentTypes`, `AgentClient`, `AgentStore`, `AgentActivity`, `AgentActivityView`, `AgentApprovalView`, `AgentDiffView`, `AgentComposerControls` in `macos/Sources/LorcaMac/`.
- Existing Swift integration: `BackendClient.swift`, `ChatStore.swift`, `Conversation.swift`, `ChatView.swift`, `ChatComposer.swift`, `ChatScrollObserver.swift`, `MiniView.swift`, `ModelPicker.swift`. ModelPicker includes manual reload; Rust lifecycle commands are reused.
- Rust: `src/agent/types.rs`, `runtime.rs`; `src/core/agent.rs`, new `agent_chats.rs`, `mod.rs`, `conversations.rs`, `generation.rs`.
- Tests: new `macos/Tests/LorcaMacTests/AgentTests.swift`; updated `BackendTests.swift`, `ChatTests.swift`, `ChatScrollTests.swift`, and `tests/agent_core.rs`.
- Docs: `README.md`, `macos/README.md`, `FUNCTIONALITY.md`, and this contract.

## Verification

- All 44 Rust tests and 34 Swift tests passed, along with formatting, strict Clippy, and the macOS build. Tests cover association, persistence, ordering/deduplication, decisions, cancellation, reconnect, and presentation.
- Live checks used a separate app identity/data directory: Qwen inspection/final response, user-approved cargo test (exit 0), command denial, cancellation, restart without replay, free scrolling/follow, normal chat, Mini status, manual reload, and an approved temporary-file diff. No application source was edited by the live agent.
- The broad acceptance request hit the existing context limit on its first attempt and timed out on its final summary after cargo test succeeded on its second. A narrower inspection completed. Model-generated summaries can still hit these limits; actual command output/exit codes remain authoritative.
- After updating, quit existing lorca clients once and relaunch the rebuilt app/CLI so the shared backend includes the updated API. `./macos/build.sh` bundles the new core; the installed terminal command updates with `cargo install --path . --locked --force`.
