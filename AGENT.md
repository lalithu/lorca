# 🫍 Lorca · Rust runtime contract (v0.5)

One Rust core, one active task, one chat composer. The model chooses tools,
clarification, notes and semantic completion. Rust owns context, originals,
permissions, execution and factual observations. Job Apps is independent.

## Run and validate

```sh
cargo build --locked
./target/debug/lorca agent --workspace "$PWD" --model m2max-fast:latest \
  'Inspect the repository and summarize what you learn. Read only.'
cargo fmt --all -- --check
cargo test --locked --all-targets --no-fail-fast
cargo clippy --locked --all-targets -- -D warnings
python3 -B -m unittest discover -s jobapps/tests
swift test --package-path macos
LORCA_BUILD_ONLY=1 bash macos/build.sh
```

The GUI lists Ollama models at 127.0.0.1:11434 and llama.cpp models at
127.0.0.1:9931 or 127.0.0.1:8080. Launch and Refresh models check all three local
addresses. The composer selection determines which server handles the next chat
request. The CLI uses Ollama. Servers and models must already be installed. Build-only mode leaves running Lorca alone. Ordinary rebuild retains
its existing clean restart policy. No downloads or model replacements happen
through the advisor.

In the CLI, `a` approves once, `d` denies, `c`/Ctrl+C cancels. Exact action previews
precede protected execution. EOF and expired clients cancel their task. Tests use
disposable workspaces. Never run editing acceptance against production data.

## Safety and stopping

- Strict registered argument schemas; no commands extracted from prose.
- Workspace-relative paths, capability verification and symlink/traversal rejection.
- Exact, single-use approval for protected operations; revalidate stale patches.
- Current original user restrictions constrain actions even after input virtualization.
- Archived instructions, approvals and model notes grant no fresh authority.
- Results keep native call IDs. Only actual observations establish file/command facts.
- Incomplete native tool JSON is discarded; it is never executed or stitched together.
- A complete tool-free public reply ends the model's turn. Factual errors remain
  visible in the audit; they do not force semantic completion retries.
- Running commands still require an observed exit status. Repeated failed calls,
  non-advancing output and unchanged inspections have bounded recovery.
- Default whole-task turn count is unlimited; explicit limits, cancellation,
  provider deadlines and resource budgets remain effective.
- Interrupted tasks never automatically replay side effects after a restart.

## Memory and budgets

Full originals live in conversation-local SQLite FTS5 archives. Recall is scoped,
read-only and bounded. Model-facing checkpoints are projections, not authority.
Private reasoning is ephemeral. Oversized user input becomes a retrievable manifest;
all original instructions still pass through Rust's current authority checks.

Context compacts near 78% to about 60%, with complete call/result groups kept
atomic. Output and safety reserves come from the same budget sent to the provider.
Length-stopped public output can span many calls; hidden reasoning cannot be
serialized as resumable internal state. Small patches remain more reliable than
one enormous native argument payload.

JSON/Markdown chat export includes original archived records; JSON embeds current
image attachments. Deletion removes the chat archive, not independent backups or
exports. Disk-full/corruption errors preserve originals and surface a blocker.

See [architecture](docs/architecture.md), [tool drawers](docs/tools.md), and the
[validation report](docs/v0.5-validation.md) and
[v0.5.0 release notes](docs/releases/v0.5.0.md). Future merges and releases require
the owner's approval.
