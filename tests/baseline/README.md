# Checkpoint A: reproducible reliability baseline

This is a **diagnostic recorder**, not the B-F implementation and not a certification
that Lorca is reliable. Production code is deliberately unchanged. Known runtime
failures remain visible in the inventory; do not change assertions to accept them.

Run on the supported macOS host with an explicitly selected installed Ollama model:

```sh
python3 -B tests/baseline/run.py \
  --output /tmp/lorca-baseline-unique-run \
  --model m2max-fast:latest --live --native
```

Choose a new output directory on every run. Without `--live` and `--native`, only
checks, builds, metadata, and the deterministic mechanics probe run. Model tasks
run serially. Native acceptance opens isolated test windows and never replaces the
installed app. The full run can take several minutes, including model preparation.

## Artifacts and interpretation

- `environment.json`: commit, dirty status, settings, hardware/toolchain, Ollama
  metadata, optional configuration presence, and sanitized running endpoint fields.
- `checks.json` and named logs: exact commands, exits, elapsed times.
- `mechanics.json`: no-evidence requirement probes, failing-test reporting,
  partial reads, a destructive-prefix proposal **not executed**, stale-content
  rejection, supported command preparation, tools and timeout scheduling.
- `live/core-live-matrix.json`: every live attempt, approvals, raw task state and
  independent checks. Individual scenario directories retain isolated project/data.
- `summary.json`: hashes of built binaries and before/after production-source hashes.

The recorder may exit successfully while it records failed checks or product
failures. Inspect the inventory and actual exits; recorder success is not product
acceptance. A normal test that records diagnostics is not a passing behavioral
regression for a known defect. The mechanics test is opt-in/ignored in ordinary
`cargo test` and explicitly invoked by this runner.

Artifacts are kept outside the repository because backend runtime directories
contain local bearer tokens. Do not commit raw runtime/data directories. Publish
only reviewed summaries. No user projects, Google accounts or production
conversations are used; no Google writes, real email or application submissions run.

The runner does not unload a model shared with other clients. It closes its own
sessions and isolated backend processes. Avoid concurrent live model tasks when
comparing runs. No extra dependencies, shell executor or runtime features are added.

## Fixtures and independent oracles

| Fixture | Evidence/oracle |
| --- | --- |
| Rust `add` subtraction bug | Actual source readback and independent offline cargo test; negative/positive scope wording |
| Small HTML slider with dark button and stray source | Actual HTML structure/visible-text inspection and Node syntax exit; hue behavior is not certified by syntax |
| File larger than 8,192-byte excerpt | Actual first/next excerpt, tail retained; prefix deletion only prepared, never executed |
| Explicit empty file | Zero byte count and no unrelated files in opt-in live test |
| Intentionally failing Rust suite | Actual failing fixture plus direct run-and-report evidence probe |
| Long-running process/cancellation | Existing `tests/agent.rs` timeout/cancellation and `tests/tool_chest.rs` process-session tests, executed in full Rust suite |
| Externally modified prepared file | Actual changed content, rejected stale execution, human content preserved |
| Pending question then cancellation | Actual same-chat Core request sequence with no edit approvals; obsolete objective independently visible |

Existing Rust tests supply deterministic process, authorization, idempotency and
cancellation coverage; this baseline does not duplicate their implementations.
The live matrix also includes explicit structured-question resumption and a precise
hue request. One diagnostic attempt per selected case is the baseline, **not** the
three-repetition final evaluation required by Checkpoint F.

## Fixture repairs separated from runtime repairs

- The existing question live test expects `WaitingForInput`, not obsolete `Incomplete`.
- Native acceptance recognizes both `exec_command` and legacy `run_command` for
  the same restricted fixture Cargo invocation.
- Long native prompts reference real `agentloop/` and tool drawer modules.
- Native acceptance defaults to explicit nominal effort; `--effort` selects it.
- Closing native acceptance does not unload other clients' model residency.

These repairs fix the tests' production contract, not agent behavior. A failed live
question or native workflow remains a failure; the runner does not weaken checks.

The official Swift test runner may be unavailable under Command Line Tools without
XCTest. Record that failure; compilation or another harness is not an XCTest pass.
No full long-context, real cloud integration, or arbitrary-model success is claimed.

`node tests/baseline/check_frontend.js /path/to/disposable/index.html` records
slider samples through DOM stubs, with an explicit hue checkpoint oracle. It is
fixture-specific, not a general HTML parser or browser. Unsupported generated DOM
APIs are recorded as oracle limitations, not silently treated as product failures.
