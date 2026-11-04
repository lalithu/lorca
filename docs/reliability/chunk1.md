# Chunk 1 implementation and verification — November 3, 2026

Base: main c5bfb1f, clean before implementation. No fetch, reset, commits, pushes, model downloads, backend migration, production installation or production restart. Changes are uncommitted. Chunks 2 and 3 were not started.

## Model and runtime

Ollama 0.40.13; m2max-fast:latest; Qwen3.5 9B Q4_K_M GGUF; model digest 0a8e78fffd91e4f7005a3ace9d6d18b21e6f3b1237e1c078549ff2e3c76cb074. Effective context remains 65,536, thinking off. Runtime-reported allocation is 7,210,312,662 bytes (6.72 GiB), not whole-system peak memory. Only the obsolete model value qwen3.5:9b-q4_K_M in /Users/deve/.config/lorca/config.json was changed to the installed alias; endpoint and all other preferences were retained. Native selection already used the installed alias.

## Context fitting

The existing Context retains token heuristics, prompt calibration, multimodal allowances and generation reserves. Fitting builds a separate candidate and restores compaction bookkeeping on failure. System/developer instructions, the exact latest user message, original/current revision instructions and Rust operational evidence are retained. Older user exchanges are removed as complete protocol units, with recent exchanges preferred. Completed older tool groups within a long task can be folded into deterministic operation facts; an unfinished call/result pair is rejected. Images remain images. No LLM summarization, new database or artifact store was introduced.

Large result payload fields are reduced inside a JSON envelope; ok, verified, exitCode, sessionId, running and correlation metadata are not inferred or clipped. originalCallId points to the unchanged result in the existing saved agentHistory. Unknown payloads receive explicitly reduced representations. Repeated reduction is stable. The original saved transcript is never rewritten by fitting. Essential-state overflow is explicit and actionable. Provider-detected overflow receives at most two fitting retries without increasing the allocated context. The older terminal conversation path now sends the same context/output budget it used for fitting.

Reproduction: on the diagnostic fixture, the previous 7,534-byte latest message lost its final constraint; it is now retained exactly. The old reduced tool result was invalid JSON and lost verified; the new result is valid JSON and retains exitCode and verified. Estimated context for that fixture changed from 194,737 before fitting / 3,928 after the old fitting to 896 after the new fitting. These are heuristic counts, not provider token measurements. The new result retains more useful structured payload (1,314 bytes versus 419 bytes in the old clipped result).

## Clarification, revisions and authority

Rust normalizes null-like clarification placeholders (None/null/nil/N/A/empty/whitespace) before validation and routing; genuine questions still pause. Explicit path spelling is restored from an unambiguous exact user reference when the model changes only letter case. This fixed a real live failure in which chosen.md could not satisfy the requested CHOSEN.md scope.

The existing Agent owns the revision counter and queue. A correction keeps the task ID, workspace, provider, completed transcript and authority continuity. It interrupts inference, invalidates pending approvals, checks revision again at action admission, discards superseded batch calls and invalidates working plans. Actions already admitted settle; asynchronous terminal sessions finish within their existing deadlines before revised effects begin. Revisions share the task's original iteration budget and are bounded to eight task versions. Scope still passes the existing Authority::continued checks; model interpretation cannot waive workspace or exact-action approvals.

The protocol adds agent_revise and agent_status. The existing composer can submit explicitly worded status/correction messages in its active owner chat through agent_start. Same-chat status is deterministic and does not trigger another inference or replace the task. Corrections and status exchanges are saved in existing conversation storage. Swift changes are submission guards only, with no layout or styling changes.

## Live coding verification

Disposable repository: /private/tmp/lorca-chunk1a/live-coding-final/project.

Real Qwen through the existing Lorca Core:

- Inspect src/lib.rs, replace erroneous subtraction with addition, approve the patch and Cargo test: completed; 15.291 seconds; final inference 2,608 prompt / 49 generated tokens.
- Follow-up referring to the preceding change and test: completed; 8.857 seconds; final inference 1,592 / 33 tokens.
- Change implementation to saturating_add and rerun Cargo test: completed; 15.941 seconds; final inference 3,527 / 38 tokens.
- During approval for creating CHOSEN.md with OLD, request status and correct the content to NEW: same task ID, old approval rejected, revised action approved, completed; 18.852 seconds; final inference 3,836 / 10 tokens. Independent readback was exactly NEW.
- Independent cargo test --offline: exit 0.

All raw sanitized task results and actual source readbacks are in /private/tmp/lorca-chunk1a/live-coding-final/results.json. Original failures are retained under live-coding and live-coding-recheck. The initial failure was the diagnostic client's Cargo allowlist using a bare name instead of the resolved absolute executable. The second attempt exposed the model filename-case defect; neither attempt was counted as successful.

These tasks are smoke tests, not broad model certification. Reported token counts describe the final inference, not total task inference; the existing interpretation pass remains a latency cost. No sustained speedup or reduced peak-memory claim is made.

## Verification

Verification logs live under /private/tmp/lorca-chunk1a/:

- complete-rust.log: final full Rust suite: 204 passed, zero failures, six ignored.
- final-swift.log: 92 passed, zero failures, with full Xcode.
- final-race-clippy.log: strict Clippy passed.
- verified-build.log: native release app built and signed; build-only, no production restart.
- jobapps.log: 111 passed, zero failures.
- restart.log: 6 passed, zero failures.
- approval-race.log: 3 revision/status tests passed, including already-granted approval invalidation.
- final-race-tests.log: real asynchronous command settling regression passed.
- final-context-results.jsonl: before/after diagnostic reproduction.
- native-acceptance.log: initial native runner compilation failed under the CLT SDK/toolchain combination.
- native-xcode-acceptance.log: full-Xcode native-window acceptance also failed because the runner still chose the CLT SDK.

Native-window acceptance subsequently PASSED using the explicit full Xcode SDK:
`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 macos/Acceptance/run.py --skip-build --model m2max-fast:latest --effort off --sdk /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk`.
Log: /private/tmp/lorca-chunk1a/native-explicit-sdk.log. Artifacts: /private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-sxxlhku7/. The runner exercised actual native views/stores, conversation switching/Mini, approved README editing, source repair, Cargo tests and follow-up. Native final screenshot was visually inspected. No production app was restarted.

An additional real-model test submitted status and correction during inference (not only at approval). It completed in 23.276 seconds under the same task ID and created CHOSEN.md containing exactly NEW. Final inference: 2,025 prompt / 10 generated tokens. Results: /private/tmp/lorca-chunk1a/live-running-final/results.json. This separate fixture still contains its deliberately seeded subtraction bug; the diagnostic's unrelated independent Cargo test returned 101. No source repair was requested in this test, and that failure is not counted as a passing build.

## Remaining boundaries

Status/correction recognition in the existing composer is deliberately explicit (for example “What are you doing?”, “Actually…”, “Correction:…”, “Instead…”, “Now change…”). The explicit agent_revise operation accepts other correction text. Ambiguous/unrelated active submissions continue to fail closed. An action already started is not rolled back. An active terminal command may delay a queued correction until its original deadline. Persistent audit records remain the full evidence; reduced context is not authority. General reasoning quality, arbitrary long-term conversational memory, and broad frontend semantic correctness are not certified by these smoke tests. Browser tools, UI redesign and Job Apps changes remain outside this chunk.

## Files changed

- `/Users/deve/Downloads/lorca/src/context.rs` — Transactional fitting, protocol validation, whole-exchange eviction and structured result reduction.
- `/Users/deve/Downloads/lorca/src/agentloop/runtime.rs` — Task revision queue, revision-safe action admission, shared iteration budget and safe resumption.
- `/Users/deve/Downloads/lorca/src/agentloop/execution.rs` — Revision checkpoints, raw correction retention, bounded overflow recovery and structured visible results.
- `/Users/deve/Downloads/lorca/src/agentloop/authority.rs` — Bind prepared actions to the current revision and reject superseded actions.
- `/Users/deve/Downloads/lorca/src/agentloop/intent.rs` — Null-like clarification normalization and exact user filename spelling.
- `/Users/deve/Downloads/lorca/src/core/agent.rs` — Explicit status and revision operations.
- `/Users/deve/Downloads/lorca/src/core/agent_chats.rs` — Same-chat active routing, idempotency and conversation persistence.
- `/Users/deve/Downloads/lorca/src/core/generation.rs` — Pass the fitted generation budget on terminal requests.
- `/Users/deve/Downloads/lorca/src/provider.rs` — Budgeted terminal request construction.
- `/Users/deve/Downloads/lorca/src/tools/mod.rs` — Default revision-settling hook for tools; unchanged implementations retain compatibility.
- `/Users/deve/Downloads/lorca/src/tools/registry.rs` — Settle resource-owning tools at revision boundaries.
- `/Users/deve/Downloads/lorca/src/tools/terminal/exec_command.rs` — Expose existing terminal session settling through the tool interface.
- `/Users/deve/Downloads/lorca/src/tools/terminal/sessions.rs` — Wait for asynchronous commands before revised effects; regression coverage.
- `/Users/deve/Downloads/lorca/macos/Sources/LorcaMac/AgentStore.swift` — Allow explicitly authorized active submissions without changing view layout.
- `/Users/deve/Downloads/lorca/macos/Sources/LorcaMac/ChatStore.swift` — Permit explicit status/correction text in the existing active-chat composer.
- `/Users/deve/Downloads/lorca/macos/Tests/LorcaMacTests/AgentTests.swift` — Active submission routing regression.
- `/Users/deve/Downloads/lorca/tests/context.rs` — Ten new context regressions in addition to existing coverage.
- `/Users/deve/Downloads/lorca/tests/agent_revisions.rs` — Pending/granted approval invalidation, identity, status and real-effect tests.
- `/Users/deve/Downloads/lorca/docs/reliability/chunk1.md` — This implementation and verification record.
