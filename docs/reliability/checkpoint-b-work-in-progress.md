# Checkpoint B stopping point — 2026-10-29

Stopped at the user’s request. B is not complete; do not start C–E from this checkpoint.

## Current work
Typed task interpretation, persisted task contracts, continuation/cancellation handling, structured clarification, scope enforcement, and completion evidence are implemented as an uncommitted candidate. Existing authorization remains enforced. No commit, push, installation, or production application restart was performed.

## Verification
The current targeted reliability_checkpoints suite compiled and passed: 4 passed, 0 failed. Log: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-stop-tests.log.

The first live Qwen candidate passed the scoped Rust repair, positive Rust repair, natural clarification/resumption, and cancel-to-new-task scenarios. Structured-question resumption failed because the model invented a read-only restriction. Subsequent corrective guards and conservative completion requirements have not been live-retested. Live evidence: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-live/.

Earlier official XCTest results (91 passed) precede these B changes and are not verification of this candidate. A release build succeeded before the latest refinements; the latest tree is compiled by the targeted tests, but a final macOS build and full regression run remain outstanding.

## Deferred work and next checks
C–E changes were separated into /Users/deve/Documents/Codex/2026-10-24/pu/outputs/b-e-deferred-snapshot/. Do not blindly apply that snapshot over B changes. The older b-e-work-in-progress.md describes the earlier checkpoint, not the current source tree.

Before declaring B complete: rerun full relevant Rust regressions and Clippy; repeat the failing live Qwen structured resumption; test scope corrections and stale constraints; review completed-task follow-up context and active-task steering; run current Swift tests and macOS build. Two finalizer test expectations were corrected after the last broader run and require that rerun. No claim of complete reliability or daily-use readiness is made.

## Approved small chunk: structured-question resumption

2026-10-29: Rebuilt the release candidate and reran the previously failing structured-question scenario with live m2max-fast:latest, nominal effort, through an isolated Core backend. It reached waiting_for_input without creating files (14.21 seconds), then completed after the answer (20.42 seconds). Independent filesystem inspection confirmed exactly CHOSEN.md, zero bytes. Evidence: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-resumption-chunk/.

No additional production behavior change was needed in this chunk: the previously added interpretation guard and prompt correction passed this attempt. Added waiting_for_filename_does_not_make_creation_read_only to tests/reliability_checkpoints.rs, exercising an incorrect read_only interpretation, corrected interpretation, and real waiting state with no writes. Existing explicit read-only enforcement regression remains passing.

Verification: reliability_checkpoints 5/5; agent_checkpoints 2/2; agent_output_limits 10/10; cargo fmt --check and git diff --check passed; cargo build --release passed. This is one live attempt, not broad model reliability certification. No Swift/macOS application build was run in this chunk. B remains incomplete. Stopped for user review; no commits, pushes, installation, or production app restart.

## Approved small chunk: stale content constraints

2026-10-29: Patch preparation now validates against the active contract’s completion requirements rather than reparsing original instruction history. Authority retains a cloned Requirements value; legacy/new tasks initialize it from their instruction. Interpreted tasks share the already-computed current requirements. Original raw prohibitions, target scope checks, action approvals, and workspace boundaries remain enforced.

Added two policy regressions: corrected content can replace an obsolete empty-file requirement while unrelated targets and commands remain blocked; active empty-file and explicit read-only constraints still reject violating edits. 3 policy unit tests plus 17 adjacent regressions passed (20 total). Release build, formatting and whitespace checks passed. Strict Clippy failed on two pre-existing B collapsible_if warnings in execution.rs and core/agent_chats.rs; no warning suppression or unrelated cleanup was applied.

Isolated live Qwen test (m2max-fast:latest, nominal) waited without writes (15.41s), then accepted a correction from empty to exactly hello and completed (16.05s). Independent readback: only CHOSEN.md, contents exactly hello. Evidence: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-corrections-chunk/.

This does not solve the separate problem of revising earlier permission prohibitions: the raw instruction gate still preserves old restrictions. Preserving unaffected constraints across further corrections also needs broader tests. Remaining proposed chunks: explicit scope/permission corrections; follow-up/context/steering behavior; final broader regressions, Clippy cleanup, Swift/macOS verification. Stop here for approval. No commits, pushes, installation, production restart, or UI changes.

## Approved small chunk: explicit permission/scope corrections

2026-10-29: Added bounded continuation authority revision. Explicit latest-user grants such as “You may now edit CHOSEN.md” replace an earlier edit prohibition for that file; “You may now run commands” revises the command prohibition only. Prior scope, forbidden files, unrelated prohibitions, workspace checks, tool preparation and per-action approvals remain enforced. Model scope can narrow inherited bounds, not broaden them. Current constraint instructions persist with the task contract so further turns do not resurrect the initial prohibition. Legacy contracts fall back to their original instruction.

Four additional policy regressions cover explicit file grants with unrelated restrictions, ambiguous/quoted/unsafe grants, command-only grants, and later continuations retaining the revised bounds. Fenced examples are ignored as permission grants. This is deliberately narrow parsing, not universal natural-language permission inference: unrecognized/ambiguous wording remains conservative. Directory-scope reconciliation also remains conservative.

Final checks: 7 policy unit tests and 17 adjacent regressions passed (24 total); release build, cargo fmt --check and git diff --check passed. Earlier strict Clippy warnings remain deferred to final verification. No Swift/macOS build in this chunk.

Live Qwen/Core test: first waiting_for_input with zero writes (15.20 seconds); then completed after explicit one-file grant (20.24 seconds). Only CHOSEN.md existed, containing exactly hello; saved scope had read_only=false, targets=[CHOSEN.md], commands_forbidden=true. The edit required allow_once approval. Evidence: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-permissions-chunk/. Live run preceded the final fenced-example guard; final guard was unit-tested and the release rebuilt afterward. One live attempt is not broad model certification.

Stopped for user approval. Two proposed B chunks remain: follow-up/context/steering behavior; final broader regressions and Swift/macOS verification with Clippy cleanup. No commits, pushes, installation, production restart, or UI changes.

## Approved small chunk: completed-task follow-up context

2026-10-29: Core now supplies the most recent completed task in the same chat and workspace as explicitly completed interpretation background. It does not create a continuation parent or import old execution evidence. Continued/corrected follow-ups retain relevant task scope; unrelated new requests do not mark completed results cancelled. Legacy metadata without a completed flag defaults to the existing pending-task behavior. The non-interpreting fallback does not append a completed objective as pending work.

Added tests/agent_followups.rs: deterministic real Core/HTTP inference and filesystem test covering creation, a pronoun-based completed-task follow-up with fresh reads/edits, tool-free status, new-topic transition, retained completed result statuses, and rejection of another submission while approval is pending. Existing Core cancellation and continuation tests remain passing. Active-task steering was inspected, not expanded: the existing busy guard remains; stop/cancel a running task before submitting a revision. No composer or UI/protocol feature was added for mid-run steering.

Verification: 26 integration tests and 7 policy unit tests passed (33 total). Release build, formatting and whitespace checks passed. No Swift/macOS application build in this chunk; strict Clippy cleanup remains in final verification.

Live Qwen (m2max-fast:latest, nominal), disposable Core backend: created CHOSEN.md containing first (14.04s); “Now change its content to exactly second” completed (13.39s), with independent readback of exactly second and only CHOSEN.md present; status completed (11.60s), no approvals or tool starts. Command prohibition and single-file scope persisted. Evidence: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-followups-chunk/. This is one live sequence, not a universal model guarantee.

Stopped for review. One planned final B verification chunk remains, subject to blockers found in broader testing. Active-task natural-language steering remains unsupported and must not be described as implemented. No commits, pushes, installation, production app restart, or UI changes.

## Verification chunk — 2026-10-29: B remains open

Broad checks on the current tree: Rust 188 passed / 0 failed / 6 ignored; official XCTest 91 passed / 0 failed; independent Job Apps 111 passed; restart 6 passed. Strict cargo clippy --all-targets -- -D warnings, cargo fmt --check, git diff --check passed. Three collapsible-if warnings in earlier B production/test changes were fixed without altering behavior. Swift tests ran before the final Rust-only inspection fix; Swift sources were not modified in this chunk.

Native acceptance initially failed because semantic Inspect target “repository” became an obligation to read a literal file named repository. Fixed by excluding abstract inspection subjects from literal read_paths; concrete file requirements remain, with a new end-to-end regression requiring actual README reading. Full Rust tests and Clippy were rerun after that change.

The subsequent native attempt passed inspection/README editing, conversation switching/Mini checks and approval, but stopped on the repair stage: model interpretation supplied clarification string “None” and the harness routed it as a real question. Native acceptance therefore FAILS; it is not waived by automated-suite success. Both attempts are retained. A rendered native screenshot was inspected and confirms the spurious waiting state; no UI source/style changes were made.

The user explicitly authorized stopping/restarting the running app in this chunk. The final app was built, signed and cleanly restarted, with data preserved (UI PID 33049 / backend PID 33041 at verification). No commit, push, CLI installation, Google write or user-project modification occurred. Temporary acceptance projects were used.

Evidence and logs: /Users/deve/Documents/Codex/2026-10-24/pu/outputs/checkpoint-b-final-verification/. Native initial/recheck raw artifacts remain in the directories recorded by their logs, outside Git.

The approved plan’s B gate includes A03–A14. Rechecking that gate revealed remaining implementation gaps beyond test cleanup: active-task status and correction handling (A10/A11), and true revision-aware invalidation of prepared actions/approvals (A14). Current incoming active requests are rejected as busy; objectiveRevision remains hardcoded 1. This is safe refusal, not implementation of those requirements. The earlier estimate of one remaining verification chunk was too optimistic.

Next bounded chunk should fix and regress the “None” clarification failure; after that, active-task revision/status and stale-action invalidation need their own implementation and acceptance checks. Do not broaden commands or mark B complete yet. Stop here for user review; C–E stay deferred.
