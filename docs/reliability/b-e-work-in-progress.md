# B-E stopping checkpoint - October 29, 2026

Stopped at the user's request. **B-E are NOT complete or release-validated.**
Base is main 08f1c0c plus Checkpoint A and the current uncommitted changes.
No commit, push, production app installation or production app restart was performed.

## Implemented so far

- B: bounded typed task interpretation through the existing inference adapter;
  relation/outcome/scope data; canonical task contract persisted on the task/result;
  separation of run-and-report from repair verification; normalization of related
  plain-text questions into WaitingForInput.
- C: partial-read tracking for large destructive edits, edit-recovery payloads in
  actual execution events, version freshness checks, task-bound approval metadata,
  canceled/denied batch-call settlement, final cancellation race check, continuation
  deduplication and superseded-task persistence.
- D: explicit timeouts, including 120, remain hard ceilings; output adapts to time;
  compact current scope is preserved without repeating the original objective;
  previous tool context is separated from current evidence for interpreted tasks.
- E: direct argv resolution for installed executables through the existing process
  sessions and macOS sandbox; task-scoped writes, explicit external-network flag,
  cleared environment, command input snapshots and legacy Cargo-only run_command.

These describe code present, not proof every required behavior works.

## Verified on the stopping tree

- `cargo build --release`: exit 0 (21.25 seconds).
- `cargo test --test reliability_checkpoints`: 7 passed, zero failures.
- `cargo fmt --check` and `git diff --check`: pass.
- Earlier library run: 32 passed, zero failures, one ignored. Additional changes
  were made afterward; do not treat that as a final full-suite result.

The seven targeted tests cover scoped target editing, read-only/out-of-target
rejection, semantic edit evidence, multi-question waiting, partial-prefix/stale
patch safety, actual Python exit 3 reporting, and actual sandbox write/network denial.

## Must resume here

1. Review the typed interpretation/authority boundary, especially adversarial scope
   wording, correction/permission changes, prior constraints, and empty-file rules.
   A model interpretation must not weaken explicit user restrictions.
2. Finish and test continuation/status/cancellation routing through Core, including
   corrections while active and restoration of the pending task after status replies.
3. Re-run all Rust suites. The last broad run was NOT green: it exposed continuation
   integration and old waiting/reporting/command fixture assumptions. Some were
   repaired afterward; the full set has not been rerun. Inspect
   `outputs/b-e-regression-detail/` in the audit workspace.
4. Review updates to existing test expectations against the plan. Preserve negative
   assertions and genuine error cases; do not accept a weaker contract to turn green.
5. Add/complete crash-boundary reconciliation, version/evidence freshness, conditional
   metadata approval and command resource/sandbox escape coverage. Saved recovery
   records are not yet a complete tested recovery experience.
6. Run serial real Qwen comparisons. The new interpretation pass has NOT yet been
   evaluated live. Do not claim Checkpoint A's live results validate this code.
7. Run official XCTest with Xcode's DEVELOPER_DIR, native acceptance, Job Apps and
   restart regressions, then document supported limits and checkpoint outcomes.

The broad suite failures and intermediate logs remain available; no failed attempts
were omitted. No new live model tasks were started after the stopping instruction.
The current source is buildable, but this is a development checkpoint, not a tested
replacement for the installed daily-use application.
