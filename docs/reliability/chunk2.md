# Chunk 2 — Native chat, thinking and trusted activity

Date: 2026-11-03. Repository: `/Users/deve/Downloads/lorca`.

## Result and scope

Implemented native thinking visibility, readable expandable tool activity, focused diff presentation, copying, inline clarification, and active-task controls. Existing SwiftUI windows, Rust agent, Core protocol, conversation persistence, context fitting and authorization remain in use. No new packages, inference backend, model, browser tools, Job Apps functionality or generated executable UI were introduced.

Started on `main` at `c5bfb1f0f60182e4b2c3793ba050037cf9d664fd`, with the expected uncommitted Chunk 1 work. Read `chunk1.md` before editing. Captured starting files/hashes and diff under `/private/tmp/lorca-chunk2/baseline`; the file inventory below compares against that baseline, not HEAD. Chunk 1-only changes remain intact. No Git history operations, production installation or production restart were performed. Build scripts were invoked with `LORCA_BUILD_ONLY=1`. Chunk 3 remains deferred.

## Native architecture and trusted data flow

Extended `ChatView`, `ChatComposer`, `AgentStore`, `AgentActivityView`, `AgentApprovalView`, `AgentDiffView` and the existing Markdown renderer. The existing timeline's typed `Content` enum is the component registry: text, thinking, grouped tool activity and compaction. Tool cards select trusted native renderers for diffs, terminal results, file reads and model plans. Status, clarification and approval reuse existing typed snapshots and Core actions.

Rust still decides task status, execution, verification, scope, revisions and approval validity. Swift derives presentation from those events. A model plan is explicitly labelled as a working plan, not completion evidence. There are no arbitrary model-defined buttons, Swift code, web views, UI execution tools or a second task state machine.

The existing text selection, native editor, conversation switching and near-bottom scroll observer remain in place. Responses use one selectable attributed `Text` per timeline text segment; code and Markdown retain existing formatting. Added whole-response/message copying, code-fence copy controls, and shared AppKit pasteboard handling. Native selected-text copying remains available through normal macOS selection/Copy. Tables retain the existing lightweight tab-separated rendering; this is not a spreadsheet layout engine.

Composer now exposes Status and Correct task during work, keeps the send control available for valid steering alongside Stop, explains rejected unrelated input, and preserves a draft when requesting status or adding a correction prefix. These controls use Chunk 1 routing. Inline clarification provides a free-text answer and existing suggested choices. Identical final/question text is not appended twice on a non-completed run.

## Thinking support and lifecycle

Path: Ollama `message.thinking` → provider `ModelUpdate::Reasoning` → `CoreInference` interception → ephemeral `TaskView.thinking` → existing agent snapshot stream → `AgentStore` → timeline `ThinkingPanel`.

The composer toggle persists through the existing `thinkingEffort` preference: OFF / nominal ON; the context menu preserves High. Current tasks keep the effort captured when started. Changing the control while active is labelled **Next task** and takes effect on the next task, not midway through inference, an approval or a continuation. Interpretation and subsequent tool-continuation requests within that task use the same captured mode.

OFF sends explicit `think:false` and drops any unexpected provider reasoning. ON uses actual `/api/show` capability values rather than model-name inference. Unsupported/unknown capability stays fail-closed with an unavailable state. Reasoning alone never enters assistant messages, parsed tool calls, task contracts, permission decisions or later model context.

Each inference has its own stable reasoning ID and states: running, complete, empty, unavailable or interrupted; OFF blocks are hidden. Provider deltas batch at 256 bytes or 50 ms on arriving frames, flush before public text/tools and finalize once. Cancellation, truncated generation and unfinished requests mark reasoning interrupted. Timeline anchors use existing event sequence and UTF-8 text offsets. No additional durable event store is used.

The panel auto-opens while reasoning and auto-collapses on completion unless manually toggled. Users can collapse, reopen and copy. Disclosure state is view-local. Text is independently bounded to 65,536 UTF-8 bytes across at most 64 blocks per Rust task; truncation is labelled. Swift retains its existing eight-run session cache. Reasoning can disappear on cache eviction, application/backend restart or history reload; it is deliberately absent from `ChatMessage`, saved conversations, audit events and model transcript. Tests check sentinel absence from these stores.

Acceptance JSON excludes reasoning. Intentional local screenshots contain actual reasoning and are the explicit visual-evidence exception; no reasoning text was copied into this report. Treat temporary screenshots as local session artifacts. No durable reasoning archive was added.

Kept the installed `m2max-fast:latest` (Qwen3.5 9B Q4_K_M GGUF via Ollama), 65,536 context and Chunk 1 generation budgeting. Base output reserve remains 8,192, recovery at most 16,384, with existing time/speed scheduling (e.g. 6,600 on the observed OFF task). No `num_predict` or context increase. Ollama's generation count includes reasoning; there is no fabricated separate thinking-token count.

## Tool visibility, diffs and interactions

Tool groups and individual rows default collapsed; pending approvals remain visible and actionable. Cards show command/file descriptions, requested/running/completed/failed/cancelled/superseded states, arguments, captured output, exit code and execution timing when supplied. Async command session completion updates the corresponding earlier command, preventing a still-running process from appearing successful. Summaries exclude failed, cancelled and superseded edits from successful file counts.

Structured argument/result metadata redacts known credential and internal session identifiers recursively. This is not a general secret scanner for arbitrary terminal output, source code or patches. Those are user-requested local artifacts and can contain sensitive text.

Output defaults to a 12,000-character preview in a bounded scroll view, with Show full captured output and Copy. This recovers what Rust captured; it cannot reconstruct bytes the execution layer already truncated. Errors and nonzero exit codes remain visible. Approval/correction state disables stale decisions while the Rust layer remains authoritative. Corrections waiting for a safe boundary are labelled; no invented progress percentage is shown.

Reused `UnifiedDiff`'s existing before/after normalization, edit comparison, reliable old/new line numbers, three-line context and expandable unchanged gaps. Added patch copying and reuse for completed edits. Existing authoritative patch text is unchanged, including full copy content. The diff renderer is equatable so unrelated stream updates do not reparse unchanged patches. Prepared patch metadata adds Created/Modified; git diffs retain `/dev/null` headers for additions/deletions. Lorca's apply_patch tool itself does not gain delete support. Multiple files/hunks remain separate. An approval's diff is rendered once rather than duplicated below its card. No model-described pseudo-diffs or Git client were added.

## Live-model findings

One small capability-metadata correction was required: the preliminary interpretation request offered only `interpret_task`, and Qwen sometimes claimed that it had no file tools. Added executor tool names as factual `availableTools` metadata plus an explicit interpretation/execution distinction. This grants no authority and changes no scope checks or task-loop design.

The final ON file summary also incorrectly claimed that the broken arithmetic test would pass. No test was executed in that read-only scenario. Completed status verifies the requested tool outcomes, not factual correctness of every sentence. The separate coding acceptance really executed cargo test after fixing the arithmetic and observed exit 0.

Retained unsuccessful acceptance attempts:

- Initial ON coding-explanation prompt proposed a source edit; fixture permissions denied it. Not a pass.
- An explicit ON read-only request stalled on the false missing-tools clarification. The capability metadata above addressed the observed confusion; subsequent ON runs read files and completed.
- An edit following a separate read-only request in the same conversation was interpreted as `correct`; retained original restrictions blocked the write. This underlying constraint/relation limitation remains. The independent editing scenario now starts in a fresh conversation. No approval bypass or authority relaxation was made to pass it.
- First standalone component screenshots had transparent backgrounds. Corrected fixture hosting backgrounds/appearance and recaptured them; those earlier captures are not visual acceptance evidence.

## Verification and measurements

| Check | Actual result |
| --- | --- |
| Full offline Rust suite | 208 passed, 0 failed, 7 ignored |
| Separately enabled identical-prompt live test | 1 passed |
| Full Swift XCTest suite | 100 passed, 0 failed |
| Job Apps Python regressions | 111 passed |
| Restart Python regressions | 6 passed |
| Strict Clippy, all targets | Passed |
| Signed native release build, build-only | Passed |
| Existing native coding/conversation acceptance | Passed, all three tasks completed |
| Chunk 2 native live UI acceptance | Passed, OFF/ON/read/correction/approval scenarios completed |
| Whitespace validation | `git diff --check` passed |

New automated checks cover ON/OFF propagation through actual Core with a mock Ollama server, reasoning exclusion from persistence/context/audit, no duplicated deltas, unsupported/empty/interrupted states, reasoning ordering with tools/public text, collapse/reopen behavior, preferences during an active task, native clipboard Unicode/code extraction, secret-key redaction, superseded summaries, large output previews, and focused large/multi-file diffs. Existing tests cover context compaction, task continuation, stale approvals, clarification, command sessions, scrolling, Mini and conversation switching.

Identical direct-provider prompt: “What is 17 multiplied by 23? Answer with the integer only.” Same installed model, temperature 0.1, no tools, context 65,536 and base output budget 8,192; only thinking mode differs. Sequential single observations, not a statistical speed benchmark:

| Mode | Wall time | Ollama generation time | Prompt tokens | Generated tokens, total | Reasoning bytes | Public answer |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| OFF | 3.894 s | 0.0576 s | 29 | 4 | 0 | 391 |
| ON | 9.992 s | 5.528 s | 27 | 266 | 703 | 391 |

Mode-specific template tokens explain the different prompt counts. Wall times include provider requests/capability checks and are not time-to-first-token. No separate reasoning token count is available. This supports keeping OFF as the default; ON is useful for inspection at additional cost, not a demonstrated general accuracy gain.

Native coding regression, OFF, isolated project: README inspection/update 13.44 s (one approved patch); arithmetic fix plus cargo test 13.67 s (two approvals, observed exit 0); contextual follow-up 8.99 s (no tools). Saved per-task usage is the last inference's usage, not an aggregate across the whole task.

The 10,000-line replacement fixture with one changed line normalized into fewer than 20 visible rows in **0.0198 s** in the final Swift run (0.0113 s in the preceding run). This measures parsing/row generation, not end-to-end frame rate. Provider reasoning batches and existing 100 ms Swift snapshot batching bound update frequency; unchanged Markdown/diffs are equatable and tool grouping is cached until events change. No Instruments frame-rate or exhaustive accessibility/mouse-gesture benchmark was performed.

Logs: `/private/tmp/lorca-chunk2/` (`rust-complete.log`, `clippy-complete.log`, `swift-final-verified.log`, `build-final-verified.log`, `jobapps.log`, `restart.log`, `live-mode-metrics.log`, `native-coding.log`, `native-ui-verified.log`). Live tests use installed tools only, fresh data/preferences and synthetic files; production accounts and workspaces are not used.

## Visual evidence

The native acceptance runner compiles real product views/stores and connects them to an isolated Rust backend and installed model. It captures native view bitmaps without screen-recording permissions. Live OFF/ON, streamed text, pending/revised approval and correction images represent actual execution. Replayed thinking uses actual captured blocks; long output, large diff, clarification/failure and component scenes are explicitly labelled fixtures, not model success claims.

Final UI artifacts: `/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw`. Visually inspected the actual full window, live streaming, separate reasoning, approvals, correction, expanded/collapsed tools, focused diff, bounded output, Mini and both appearances. Native fixture windows now use explicit opaque backgrounds.

Final live UI measurements below are **different tasks**, so their latencies are not an ON/OFF speed comparison. Timings are captured at task settlement before screenshot encoding; generated tokens are last-request usage only.

| Scenario | Settlement time | Reasoning bytes | Last request generated tokens | Result |
| --- | ---: | ---: | ---: | --- |
| OFF explanatory response | 13.260 s | 0 | 411 | Completed |
| ON read two files/summarize | 23.358 s | 2,718 | 331 | Completed; three separate reasoning blocks, two actual read_file calls |
| OFF approved edit/correction | 15.544 s | 0 | 26 | Completed; old approval unusable; exact corrected README readback passed |

- [Live OFF streaming](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/thinking-off-streaming.png)
- [Live ON reasoning](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/thinking-on-live.png)
- [ON final: reasoning separate from tools/answer](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/thinking-on-final.png)
- [Replayed actual thinking expanded](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/thinking-expanded-replay.png)
- [Replayed actual thinking collapsed](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/thinking-collapsed-replay.png)
- [Live pending approval with real diff](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/pending-approval-live.png)
- [Live status/correction](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/correction-live.png)
- [Actual tool cards expanded](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/tool-cards-expanded.png)
- [Actual tool cards collapsed](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/tool-cards-collapsed.png)
- [Focused 1,000-line diff fixture](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/focused-large-diff-fixture.png)
- [Long terminal fixture](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/long-terminal-fixture.png)
- [Inline clarification/failure fixture](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/clarification-failure-fixture.png)
- [Light component rendering](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/components-light.png)
- [Dark component rendering](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/components-dark.png)
- [Light conversation](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/conversation-light.png)
- [Mini window](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-cp9h45sw/mini.png)

Original coding evidence: `/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-jvtjlp6s` (`native-report.json`, `native-final.png`). Temporary artifacts are not checked into Git.

## Files changed in this chunk

Paths below are relative to `/Users/deve/Downloads/lorca`; Chunk 1-only modified files are deliberately excluded.

| File | Purpose |
| --- | --- |
| `src/agent_types.rs` | Ephemeral reasoning update/block and optional TaskView field |
| `src/provider/agent.rs` | Capability-aware ON/OFF, batched reasoning stream and lifecycle tests |
| `src/core/agent.rs` | Forward presentation reasoning independently of executable turns |
| `src/agentloop/runtime.rs` | Bounded session reasoning, anchors and interruption |
| `src/agentloop/execution.rs` | Route existing tool names into interpretation; exhaustive update handling |
| `src/agentloop/intent.rs` | Executor capability facts for preliminary interpretation |
| `src/tools/files/apply_patch.rs` | Created/Modified presentation metadata |
| `macos/Sources/LorcaMac/AgentTypes.swift` | Thinking decoding, queued correction predicate, safe structured metadata |
| `macos/Sources/LorcaMac/ThinkingPanel.swift` (new) | Native reasoning disclosure/lifecycle |
| `macos/Sources/LorcaMac/AgentActivity.swift` | Accurate terminal/superseded states and merged reasoning timeline |
| `macos/Sources/LorcaMac/AgentActivityView.swift` | Typed components, collapsed tools, inline answers, copies and results |
| `macos/Sources/LorcaMac/AgentApprovalView.swift` | Focused diff reuse, file-kind label and metadata redaction |
| `macos/Sources/LorcaMac/AgentComposerControls.swift` | Persisted ON/OFF control and next-task semantics |
| `macos/Sources/LorcaMac/AgentDiffView.swift` | Equatable diffs, patch/output copying, bounded recoverable previews |
| `macos/Sources/LorcaMac/MarkdownText.swift` | Selectable existing renderer, whole/code copying, equatable rendering |
| `macos/Sources/LorcaMac/ChatComposer.swift` | Status/correction/send controls during work |
| `macos/Sources/LorcaMac/ChatView.swift` | Inline answer dispatch and accurate generation indicator |
| `macos/Sources/LorcaMac/AgentStore.swift` | Disable decisions while correction is pending |
| `macos/Tests/LorcaMacTests/Chunk2Tests.swift` (new) | Eight presentation, disclosure, clipboard and state tests |
| `tests/thinking_presentation.rs` (new) | Core ON/OFF integration and ephemeral-data regression |
| `tests/agent_thinking_live.rs` | Opt-in identical-prompt local ON/OFF measurements |
| `macos/Acceptance/Chunk2Acceptance.swift` (new) | Live UI scenarios and deterministic visual fixtures |
| `macos/Acceptance/AgentAcceptance.swift` | UI acceptance dispatch and exclude reasoning from disk report |
| `macos/Acceptance/run.py` | UI mode and native helper compilation |
| `macos/Acceptance/README.md` | Safe reproduction instructions and evidence/privacy distinctions |
| `docs/reliability/chunk2.md` (new) | This report |

## Remaining limits

Model interpretation and generated prose remain fallible, including the retained read-only-to-edit relation issue above. This chunk does not claim universal agent correctness or fix that scope behavior. Reasoning visibility does not elevate reasoning to execution evidence. Large captured outputs still inherit Rust truncation limits. Arbitrary log/code secrets are not automatically detected. Syntax highlighting remains the existing monospace/added/removed styling. Selection/clipboard and scroll behavior have automated native coverage, but exhaustive manual selection gestures, VoiceOver and every window size were not tested. Mini keeps its compact transcript; detailed tool/approval work is opened in Chats. A correction stays attached to the original task card, so later intervention messages can appear below that card. No durable reasoning reload is promised.

All work remains uncommitted. Stop at Chunk 2; browser automation and further reliability redesign require a separate task.
