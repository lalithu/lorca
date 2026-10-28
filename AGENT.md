# 🫍 lorca · Rust runtime and interface contract (v0.4)

- One Rust core, one active task; agent execution is embedded in existing Chats.
- The Job Apps sidebar has a separate research workflow; this runtime contract covers Chats. No MCP, subagents, or background service installation.

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
- `read_file`: verified UTF-8 file, read in bounded 4–8192 byte excerpts using `offset`, optional `length`, and `nextOffset`. Reads can exceed 128 KiB in total; patches retain the 128 KiB limit.
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
- Tool output is bounded; commands capture 16 KiB. The shared Rust context manager estimates model input, includes schemas and generation headroom, and compacts near 78% of allocated capacity. There is no unconditional 64 KiB working-history failure.
- Uses Ollama native structured calls, full assistant call messages, and correlated tool results. No parsing executable actions from prose or exposing private thinking.
- Ordinary text answers complete without tools. Common unsupported action claims are rejected; unresolved failed/denied edits and commands remain incomplete. Model prose never supplies permission or execution evidence.
- Only the latest task runs in memory. Live UI events remain bounded to 300 / 2 MiB; full structured turns and execution events are saved on the conversation response. Interrupted work is not resumed or replayed after restart.

## Chats

- One composer handles text, images, and optional workspace tools. No Chat/Agent selector or routing inference request.
- Expand tool activity for arguments/results; commands and patches require exact-action inline approval.
- Scroll up to pause following; return to the bottom to resume. Mini shows basic status and opens Chats.
- Requests, streamed partial text, final results, structured calls/results, and execution activity save in the originating conversation. Drafts, images, and IDs remain compatible.
- The live backend snapshot is bounded; saved activity can be restored from conversation metadata. Swift keeps an eight-run presentation cache and batches updates every 100 ms.
- Backend restarts mark unfinished saved requests interrupted; they are never resumed or resubmitted automatically.
- GUI model selection is lazy: opening the app or changing selection does not preload. Submitted work prepares the model. Model menu → Unload model / Reload model uses shared-model warnings and Rust leases; stop active work first. No automatic reload.
- GUI safeguards remain 24 model turns, 120 seconds per inference request, 120 seconds per command. Context compacts between complete turns; actual time/iteration limits remain explicit.

## v0.4B interface contract

Use the existing authenticated local backend (`Endpoint`, bearer token, no browser Origin):

- `POST /command`: `{"op":"agent_start","session":"…","task":"…","workspace":"/absolute/path","model":"qwen3.5:9b","limits":{"iterations":20,"modelSeconds":120,"commandSeconds":30}}` → `{"taskId":"…"}`. Workspace/model/limits are optional. Without a workspace the tool registry is empty. Chats also sends `conversationId` and a unique `requestId`; the saved request ID prevents duplicate execution after an uncertain acknowledgement.
- `agent_snapshot` + `session` → latest `TaskView` or `null`.
- `GET /agent/events` + `X-Lorca-Session`: NDJSON `TaskView` snapshots, including current approval and bounded event history. Reconnect with `agent_snapshot`; deduplicate by task ID + event sequence. A sequence gap means older activity rolled off.
- `agent_decide` + `session`, `taskId`, `approvalId`, `decision` (`allow_once`, `deny`, `cancel`). Tokens are exact-action, owner-session, single-use; never send edited action arguments.
- `agent_cancel` + `session`, `taskId`. Closing/expiring the owner session cancels too.
- Status: `running`, `awaiting_approval`, `completed`, `incomplete`, `denied`, `cancelled`, `failed`.
- Events: `task_started`, `model_generating`, `assistant_message`, `context_compacted`, `tool_requested`, `approval_required`, `tool_started`, `tool_completed`, `tool_failed`, `file_changes`, `terminal_output`, `task_completed`, `task_incomplete`, `task_denied`, `task_cancelled`, `task_failed`, `task_continued`, `finalization_requested`.
- Each event has `id`, `taskId`, monotonic `sequence`, `kind`, and structured `data`. Calls have stable `callId`; approvals contain complete immutable `action` details. Terminal output identifies stdout/stderr.
- `TaskView` includes optional `conversationId` / `requestId`, the model name, a monotonic revision, cumulative public `assistantText`, a `generating` flag, and confirmed `usage`. `assistant_message` contains only public assistant content, never private thinking. SwiftUI projects these events in Chats; all execution and permission enforcement stays in Rust. Automatic restart/resume is not implemented. Saved execution history does not authorize or replay operations.

## Layout

- `src/agentloop/`: provider-independent runtime, permissions, tools, workspace access, command sandbox, event/task types.
- `src/provider/agent.rs`: native Ollama tool-call adapter.
- `src/core/agent.rs`, `agent_chats.rs`: existing model lifecycle/session integration, saved request/result association, and interruption recovery.
- `src/agent_cli.rs`: temporary testing interface.
- `tests/agent*.rs`: deterministic runtime, permission, filesystem and core integration tests; command sandbox tests live alongside the runner.

## Changed-file map

- New Swift agent modules: `AgentTypes`, `AgentClient`, `AgentStore`, `AgentActivity`, `AgentActivityView`, `AgentApprovalView`, `AgentDiffView`, `AgentComposerControls` in `macos/Sources/LorcaMac/`.
- Existing Swift integration: `BackendClient.swift`, `ChatStore.swift`, `Conversation.swift`, `ChatView.swift`, `ChatComposer.swift`, `ChatScrollObserver.swift`, `MiniView.swift`, `ModelPicker.swift`. ModelPicker includes manual reload; Rust lifecycle commands are reused.
- Rust: `src/agentloop/types.rs`, `runtime.rs`; `src/core/agent.rs`, new `agent_chats.rs`, `mod.rs`, `conversations.rs`, `generation.rs`.
- Tests: new `macos/Tests/LorcaMacTests/AgentTests.swift`; updated `BackendTests.swift`, `ChatTests.swift`, `ChatScrollTests.swift`, and `tests/agent_core.rs`.
- Docs: `README.md`, `macos/README.md`, `FUNCTIONALITY.md`, and this contract.

## v0.4 context, streaming, and grounding

- `src/context.rs` is shared by the existing terminal conversation path and the
  unified tool loop. Token estimates use Unicode text heuristics (not serialized
  byte length), schema overhead, image allowance, and confirmed prompt calibration.
- The allocated window comes from Ollama `/api/ps`; an unavailable allocation uses
  a conservative 16,384-token request budget without presenting it as confirmed UI data.
- Compaction keeps recent complete tool groups and images, builds a deterministic
  structured checkpoint, and retains Rust-owned objective, references, operation
  outcomes, denials, and unresolved errors. It makes no extra model requests.
- Checkpoints are user-role historical data, never elevated system instructions.
  Oversized observations shrink only in working memory; original results stay saved.
- Incomplete tool groups from interrupted requests receive explicit unconfirmed
  results when forming later context. No side effects are replayed by this repair.
- `src/provider/agent.rs` assembles fragmented NDJSON/UTF-8, public deltas, and native
  indexed/ID-correlated calls. Missing completion acknowledgement fails without
  executing calls; already streamed text stays visible and saved.
- Confirmed usage includes prompt tokens, generated tokens, generation duration,
  and tokens/second. Compaction clears obsolete metrics until a new confirmed turn.
- `Workspace` has a bounded verified reference set populated from real root listing,
  directory/search results, and saved successful discovery. Paths are resolved again
  inside the capability before reuse; edits keep existing symlink/stale-source checks.
- Successful duplicate actions in an active run reuse their recorded result. A file
  mutation invalidates command-result reuse, allowing tests to run again after a fix.
  Repeated unchanged failures stop early; denials remain task scoped.

## Verification and reproduction

Required checks:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test --package-path macos
python3 -B -m unittest discover -s jobapps/tests
./macos/build.sh
```

Rust checks, the Swift XCTest suite, Python tests, and native app builds have passed
on the development Mac with full Xcode. Tests use isolated storage, temporary
workspaces, local servers, and deterministic approvals where appropriate; they do
not unload real Ollama models.

Earlier live acceptance with local Qwen verified streaming, discovery, an approved
fixture edit, approved sandboxed Cargo tests, and contextual follow-up. Those runs
are distinct from deterministic tests and do not guarantee arbitrary model output.
See [native acceptance](macos/Acceptance/README.md) for the reproducible runner.

Relaunch app/CLI clients after rebuilding so they use the current protocol and core.
The build script selects full Xcode when available and handles the known macOS 26.5
command-line-tools SDK fallback. No background service installation is required.

## Reliability stop checks

- Explicit action requirements are checked against this request's real results.
  Named read/edit targets stay separate; requested Cargo commands need an execution
  record after the latest actual edit. A nonzero exit proves execution, not success.
  Generic greetings, explanations and contextual result questions need no tools.
- A model-only promise cannot complete an edit. At most two consecutive corrective
  continuations are inserted into the same task, with unverified outcomes and tool
  guidance. Newly verified work resets this counter; global limits stay unchanged.
- Empty replies after satisfied, settled tools get one tool-free finalization.
  Any calls returned during that phase are rejected without execution. If work is
  still missing, bounded continuation retains the original tool registry instead.
- Denial ends the task as Denied immediately. Failed operations remain Failed;
  unmet goals or exhausted continuation become Incomplete. All terminal states
  persist without automatic replay or being relabeled interrupted on restart.
- Workspace initialization discovers up to 200 real entries through depth two,
  bounded to 100 ms, excluding hidden/build/vendor folders and symlinks. The initial
  prompt inventory is capped at 6000 bytes. Unknown paths receive bounded relevant
  suggestions, and existing capability, traversal, symlink and stale-source checks remain.
- Completion evidence and executed-action identities live outside compacted model
  messages. Operational checkpoints deduplicate unchanged reads; the complete saved
  transcript/audit retains every call. Pending approvals settle before compaction.
- Command observations preserve exit/error metadata when output is shortened.
  Source-derived prose remains model output: execution evidence does not certify
  every semantic claim in generated documentation or arbitrary code.

### Reproduce native acceptance

See `macos/Acceptance/README.md` and `python3 macos/Acceptance/run.py --help`.
The runner uses the real Dashboard/Mini, composer, stores and local backend in an
isolated fixture. It does not replace the XCTest suite or manual user acceptance.

Reference mechanisms: [Codex turn-stop continuation](https://github.com/openai/codex/blob/main/codex-rs/core/src/session/turn.rs),
[Hermes intent acknowledgment recovery](https://github.com/NousResearch/hermes-agent),
[OpenClaw settled-tool finalization](https://github.com/openclaw/openclaw), and
[Aider's bounded repository context](https://aider.chat/docs/repomap.html).
No agent framework, dependency, model router or second verifier was added.

The local transport protocol is now 3 because Incomplete/Denied extend the native outcome contract. Relaunch app/CLI clients after rebuilding; older backends are rejected before accepting a task.

## Four-pillar task contract

The existing loop initializes one Rust-owned TaskContract per task, retaining the
original instruction, objective requirements, bounded context references and task
restrictions. Authority checks run before tool preparation/approval; all existing
workspace and approval enforcement remains. Verification reports real execution
records, never model claims; a nonzero required command does not satisfy success.
The additive task_contract and task_verification events use existing audit storage.
No wire fields, conversation IDs or persisted message schema were removed.

`src/tools/` now owns native tool definitions, registry, validation, workspace and
command execution. `lorca::agent` remains an API alias. See
[architecture and limitations](docs/architecture.md) for module responsibilities,
compaction behavior and the deliberately limited restriction recognizer.

### Small-window checkpoint representation

Compaction pins the original instruction once and projects compact per-file read
facts and operation metadata rather than re-serializing the full contract. Full
objective/restrictions, approvals, execution caches and verification history stay
in Rust and the saved audit. Old generated checkpoints are not recursively
summarized; narrative yields space to essential state. Initial discovery inventory
is replaced by a compact verified-file index, while capability references remain
in Rust. Provider and context manager share the generation reserve: an allocated
8192-token window leaves 6144 input tokens after reserving 2048 for output, including
schemas and system text. UI usage is the last confirmed measurement, not a guarantee
that the next prompt fits. Genuine essential-state overflow remains an error with
explicit budget diagnostics; it is not silently truncated.
