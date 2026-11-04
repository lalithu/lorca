# Chunk 2 final UI fixes — 2026-11-03

Implemented the requested native presentation fixes on `/Users/deve/Downloads/lorca`, preserving existing uncommitted work on `main` at `c5bfb1f0f60182e4b2c3793ba050037cf9d664fd`. Starting source copies and hashes are in `/private/tmp/lorca-chunk2-fixes/baseline`. No commits, pulls, resets, installation, production restart, new packages or Chunk 3 work.

## Changes

- **Thinking ON color:** the brain and “Thinking ON” label use fixed sRGB `#22C55E`. OFF uses neutral primary styling. The label reads the existing published preference directly, so it updates with the toggle; no Core acknowledgement is required. “Next task:” stays neutral while the selected ON label/icon remain green. No composer-wide tint or background change. The same green is used in light/dark appearances.
- **Thinking prose:** `ReasoningText` replaces terminal-output reuse. Text wraps within the viewport; a non-interactive geometry measurement sizes short content naturally and caps long content at 180 points with vertical scrolling. Text selection and Copy thinking remain available. Terminal/code/diff horizontal scrolling is unchanged.
- **Stage labels:** interpretation explicitly invokes a presentation-tagging adapter entry point, with the same messages, tools, budget, effort, provider and inference call as before. Typed `ThinkingStage` metadata flows to the ephemeral reasoning block. Swift renders “Understanding request” for interpretation and “Thinking” for execution. Retries/continuations remain separate, chronologically anchored blocks in the existing task timeline. Completed blocks collapse automatically unless overridden. No stage guessing from generated text or panel count.
- **Offsets:** `AgentPresentation.responseText` preserves the original assistant string. Only timeline text segments are trimmed for display, after raw UTF-8 offsets have been applied. Tests cover leading whitespace, Unicode, multiple tools and stable streamed row IDs.
- **Disclosure:** manual choices live in a presentation-only map on the existing `AgentStore`, keyed by task/request IDs. They survive row recreation and chat switching, expire with the eight-run cache, and are pruned when blocks leave a snapshot. They are not persisted and do not duplicate Rust task state. Standalone panels also respect manual choices on reappearance.
- **User bubbles:** the shared existing bubble layout now uses native system blue with white attributed text in Chats and Mini. Short messages keep natural proportions; long/multiline messages wrap. The permanent Copy message row is removed from user content. Whole-message copying is on the bubble background context menu so it does not own the selectable text's gesture. Native selected-text Copy remains distinct. Code-copy buttons remain available.
- **Clarification:** the authoritative question is presented once with its existing answer controls. Matching response text is suppressed after timeline construction, preserving offsets and other failure/progress text. Expanded question-tool details point to the conversation instead of repeating question arguments/result JSON. Tool groups remain collapsed by default; terminal failures/exit codes remain visible.

## Two-stage inference remains unchanged

Interpretation and execution both run and both emit actual reasoning when ON. `generate_interpretation` delegates to the same generation implementation, attaching only an enum to reasoning snapshots. No inference prompts, routes, model choice, Ollama settings, context/output budgets, context fitting, tool admission or authority rules changed. The provider and execution-loop source files are unchanged from this task's starting baseline.

The Core mock-Ollama regression verifies exactly four requests across ON/OFF: one interpretation and one response in each mode, explicit think true/false, context 65,536, correct stage metadata, and no reasoning in transcript/audit/saved conversation data. Additional live tool continuations remain execution-labelled.

## Verification

| Check | Result |
| --- | --- |
| Full Rust suite | 208 passed, 0 failed, 7 ignored |
| Final focused Core/stage/request-count regression | 1 passed |
| Final complete Swift suite | 106 passed, 0 failed |
| Strict Clippy, all targets | Passed |
| Signed native release build | Passed, build-only; production not restarted |
| Existing live native UI acceptance | Passed |
| Whitespace / Rust formatting | Passed |

Six new Swift tests cover the diagnosed offset, disclosure/cache, stage decoding, clarification, wrapping and green-color requirements. Existing context/revision/approval regressions remain in the full Rust suite. The complete Swift suite includes existing Job Apps regressions; no Job Apps source was changed. No new broad speed benchmark or frame-rate claim is made.

Native UI acceptance used the full Xcode SDK with isolated storage/preferences and the installed `m2max-fast:latest` model. OFF produced no reasoning text. ON completed a read-only file task with real interpretation and execution blocks, and changing the selected effort mid-request retained existing next-task behavior. Manual disclosure survived actual new-chat/open-original navigation. The approved edit/status/correction scenario invalidated the old approval, approved the revised action and verified exact corrected README readback. Full Chats, Mini, expanded/collapsed tools, focused diffs, clarification/failure and light/dark states were captured and visually inspected.

Live UI evidence directory:
`/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531`.

- [Separate trusted stage labels](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531/thinking-on-final.png)
- [Reopened wrapped reasoning after chat switch](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531/thinking-reopened-after-chat-switch.png)
- [One clarification question with failure retained](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531/clarification-failure-fixture.png)
- [Light conversation / blue bubbles](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531/conversation-light.png)
- [Mini](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-0j9ll531/mini.png)

## Native mouse selection and appearance acceptance

Added `--selection` to the existing runner. It runs actual product components under a real `NSApplication.run()` event loop with a standard Edit/Copy key equivalent. It posts mouse-down/drag/up and Cmd+C key events into its own windows. It never assigns selectedRange or substitutes a clipboard write for the tested copy. Results record copied substrings, actual AppKit text-interaction responder, multiline selections and native Copy-menu availability. Original pasteboard representations are restored after the run. All fixture content/storage is disposable.

Earlier attempts that missed right-aligned or smaller text, reused old field editors, or selected only a newline are retained under `/private/tmp/lorca-chunk2-fixes/selection*.log`; they are not counted as passing multiline verification. The final probe uses actual accessibility text bounds when available, real hit testing, fresh windows and both drag directions. At least two nonempty copied lines are required to count a multiline pass.

At 360- and 850-point fixture widths, reasoning document/viewport widths measured 328/328 and 818/818 points respectively, with the scroll viewport capped at 180 points. This directly verifies that long reasoning no longer creates a huge horizontal document. Light and dark fixtures show the same green ON brain/text, neutral OFF controls and neutral “Next task:” prefixes. Blue user bubbles have white text and no permanent message-copy button.

Final selection artifacts: `/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj`.

| Native text component | Drag + Cmd+C | Two nonempty copied lines | Native Copy menu |
| --- | --- | --- | --- |
| assistant | Verified | Verified | Verified |
| user | Verified | Unverified | Verified |
| reasoning | Verified | Verified | Verified |
| tool-output | Verified | Verified | Verified |
| code | Verified | Verified | Verified |

**Remaining verification limit:** the blue user bubble produced genuine selected substrings and its native Copy menu, but automated cross-line drags did not reliably copy two nonempty lines. This is not counted as passing multiline user-selection acceptance. No general claim that every selection gesture works is made.

- [Green ON / neutral OFF and queued labels, light](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/thinking-green-light.png)
- [Green ON / neutral OFF and queued labels, dark](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/thinking-green-dark.png)
- [Blue user bubble, light](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/user-blue-light.png)
- [Blue user bubble, dark](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/user-blue-dark.png)
- [Actual assistant multiline selection](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/assistant-multiline.png)
- [Actual reasoning multiline selection](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/reasoning-multiline.png)
- [Narrow wrapped reasoning](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/thinking-wrap-narrow.png)
- [Wide wrapped reasoning](/private/var/folders/c8/3355sdl92s5dl4w8j6g7ps280000gt/T/lorca-native-acceptance-adqk2mcj/thinking-wrap-wide.png)


## Files changed in this task

Relative to `/Users/deve/Downloads/lorca`, compared with the starting baseline (not the older Git HEAD):

| File | Purpose |
| --- | --- |
| `src/agent_types.rs` | Typed stage and compatible presentation-tagging inference hook |
| `src/agentloop/intent.rs` | Explicit interpretation-stage hook, same request/behavior |
| `src/core/agent.rs` | Attach trusted stage while forwarding actual reasoning |
| `src/agentloop/runtime.rs` | Retain stage on ephemeral block |
| `tests/thinking_presentation.rs` | Stage, same request count and privacy regression |
| `macos/Sources/LorcaMac/AgentTypes.swift` | Backward-compatible optional stage decoding and labels |
| `macos/Sources/LorcaMac/ThinkingPanel.swift` | Wrapping, naturally sized prose, safe hit testing and disclosure |
| `macos/Sources/LorcaMac/AgentStore.swift` | Bounded cached manual disclosure choices |
| `macos/Sources/LorcaMac/AgentPresentation.swift` (new) | Pure source-preserving response/clarification helpers |
| `macos/Sources/LorcaMac/AgentActivityView.swift` | Correct source offsets, cached disclosure and one question |
| `macos/Sources/LorcaMac/AgentActivity.swift` | Readable question/plan group labels |
| `macos/Sources/LorcaMac/AgentComposerControls.swift` | Green selected ON label/icon, neutral queued prefix/OFF |
| `macos/Sources/LorcaMac/MarkdownText.swift` | White user text and no permanent user-message copy row |
| `macos/Sources/LorcaMac/ChatView.swift` | Shared blue native bubble and background copy menu |
| `macos/Tests/LorcaMacTests/Chunk2FixTests.swift` (new) | Six focused regression tests |
| `macos/Acceptance/Chunk2Acceptance.swift` | Live stage/disclosure assertions and screenshot |
| `macos/Acceptance/SelectionAcceptance.swift` (new) | Native drag, Cmd+C, menus, color and resize fixtures |
| `macos/Acceptance/run.py` | Select the native selection entry point |
| `macos/Acceptance/README.md` | Reproduction/evidence notes |
| `docs/reliability/chunk2-fixes.md` (new) | This report |

## Limits and boundaries

Reasoning remains session-only, subject to the existing cache and byte/block caps. Model prose can still be wrong, and two-stage latency remains intentional. No browser work, inference optimization, persistent UI database or event-protocol redesign was introduced. Native interaction tests use generated AppKit events in real product text components; they are not physical hardware/VoiceOver testing or certification across all macOS versions. Context-menu verification records the native menu's Copy item; every menu-item interaction was not manually exercised. Screenshots intentionally contain local-model reasoning for visual evidence and remain temporary local artifacts.

Verification logs are in `/private/tmp/lorca-chunk2-fixes/`: `rust-tests.log`, `rust-stage-final.log`, `clippy-final.log`, `swift-verified.log`, `build-verified.log`, `native-ui.log`, and selection logs. All work remains uncommitted; stop at Chunk 2.
