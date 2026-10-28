# Runtime architecture

SwiftUI Dashboard and Mini share the authenticated Rust core with the terminal client.
`agentloop/` contains the existing single loop; `tools/` contains the native registry,
argument validation, workspace capabilities and sandboxed command implementation.
Shared protocol/tool types live in `src/agent_types.rs`, avoiding a tools-to-runtime dependency.
The public `lorca::agent` path remains a compatibility alias for `agentloop`.

## Four pillars

- **Objective:** original instruction, requested outcome, restrictions and existing deterministic completion requirements. Ambiguous intent retains the original text; there is no parsing inference call.
- **Context:** conversation/workspace references plus the existing bounded working-context engine in `src/context.rs`. The full contract is included in operational checkpoints; execution evidence remains outside compacted messages.
- **Authority:** task-local instruction restrictions are checked before preparation and approval. The existing workspace boundary, strict argument schemas, Cargo allowlist and immutable Allow Once gateway remain authoritative. Model/tool text cannot modify the contract.
- **Verification:** real task transcript records drive a structured report of unmet requirements and supporting tool IDs, paths, verification flags and exit codes. Required commands must exit zero after relevant edits; an executed failing command does not verify success.

## Flow

Submit → initialize contract → assemble bounded context → stream Ollama response → validate tool and task authority → request exact-action approval where necessary → execute → record evidence → verify → continue or finish.

The existing duplicate-action cache, denial stop, repeated-failure limits, cancellation,
model/command deadlines and iteration limits remain in place. Existing transcript and
audit persistence also retains the additive task_contract/task_verification events;
no new database or restart replay is introduced. Providers, storage and transport
remain separate. SwiftUI does not enforce execution permissions.

## Limits

Natural-language restriction recognition is deliberately conservative and limited to
explicit common prohibitions against editing or command execution. A named edit ban
currently blocks all patches for that task. Other restrictions remain recorded but
are not a general language policy engine. Unknown intent never bypasses tool checks
or approvals. Scope changes require a new user task; no model-driven scope updates.
Verification establishes tool outcomes, not arbitrary semantic correctness of prose
or code. Existing unresolved-error and unsupported-claim gates also prevent false
completion. Unfinished work is not automatically resumed after restart.

See [AGENT.md](../AGENT.md) for runtime limits and the local API contract.

## Context budgeting

The model-facing checkpoint is a projection, not the full task-contract event.
The objective appears once; per-file read facts are coalesced and duplicate operation
facts/cache identities are omitted. The complete contract, capability references,
execution/denial caches and verification transcript remain authoritative in Rust.
Previous generated checkpoints are never recursively summarized into newer ones.
Initial discovery text is removed from the system prefix after compaction; a compact
verified-file index remains available and additional paths can be rediscovered.
Narrative summaries shrink according to remaining budget before essential state
can be rejected. Tool call/result groups and original image inputs remain intact.
Ollama's effective allocation is read from /api/ps, not model maximum metadata.
At 8192 capacity, 2048 output tokens leave 6144 input tokens including schemas and
system instructions. The UI reports last confirmed usage, not the next prompt's
estimated size. Both provider and context manager share the output-reserve function.
