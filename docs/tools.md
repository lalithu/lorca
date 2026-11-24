# Tool chest

`src/tools/mod.rs` defines the small `Tool` interface and prepared action. `registry.rs` owns the handlers, schemas, descriptions, drawer categories and required authority. Each drawer registers its own working handlers:

| Drawer | Modules |
| --- | --- |
| Files | `read_file`, `list_directory`, `search_files`, `apply_patch`, `git_diff` |
| Terminal | `exec_command`, `write_stdin`; shared sandbox/process implementation in `command.rs`, task-local sessions in `sessions.rs` |
| Vision | `view_image` |
| Web | `fetch_url`, `web_search`; shared bounded public-HTTPS transport in `http.rs` |
| Agent | `tool_search`, `update_plan`, `request_user_input`, `request_permissions`, `search_history`, `get_history_record`, `compact_context` |

## Discovery and authority

Before a folder is selected, the normal registry exposes `tool_search`, `request_workspace`, and `request_user_input`. With an active workspace it exposes `tool_search`, `request_user_input`, `list_directory`, `read_file`, `search_files`, `apply_patch`, `exec_command`, and `write_stdin`. Configured `web_search` is also initially visible in either state; unavailable integrations are never exposed. Other tools remain discoverable, up to five schemas per search, without a classifier or routing request. Discovered schemas stay visible within a task and are restored from linked tool history on follow-ups, using only tools still registered for the current workspace. Invalid calls return the exact schema and a correction hint. Discovery never grants permission; workspace restrictions, approvals and bounded execution still apply. Custom registries, including Job Apps, keep their own tool selection.

All calls still go through `agentloop/authority.rs`: task restrictions, preparation, single-use human approval when required, cancellation, execution, and real result evidence. Files retain capability-based workspace boundaries, discovered references and atomic patch verification. Plans do not count as completion evidence. Explicit question tools pause in a waiting state; the next user message provides the answer. Permission requests pause and never grant access by themselves. `request_workspace` can propose a known absolute folder. Confirm it or choose a folder to resume the original request, with the new working directory supplied as structured context. Paths inside the active workspace can be absolute or relative. An outside path requires a new user-confirmed workspace.

## Terminal

`exec_command` accepts an executable name or path, an argv array, and a working directory inside the workspace. It runs ordinary commands through the macOS sandbox after approval, with a temporary home, bounded output, and the exact reviewed argv. `read_only: true` blocks workspace writes. `network: true` grants outbound access. `git_metadata: true` permits Git metadata writes, or linked worktree metadata reads when combined with read-only mode. These options default to false. Shell commands need explicit argv such as `{"executable":"sh","arguments":["-c","printf hello"]}`. Legacy `run_command` retains its Cargo-only schema and wait-until-complete behavior.

Use `git_diff` for tracked changes, with `staged: true` for the index. Use `exec_command` with `git status` or `git log` for other inspection. Linked worktrees whose Git metadata is outside the active workspace require review of the extra metadata scope. User-denied actions get one tool-free reply for a revised suggestion or blocker, with further action requiring a new user turn.

A running command returns `sessionId`. Poll with `write_stdin` using `session_id` and optional `yield_time_ms`. Supply `chars` (up to 4096 bytes) and/or `close_stdin` to interact; these writes require a fresh approval, while empty polling does not. Output is bounded to 16 KiB per command, with a real exit code when finished. The original command timeout remains in force between polls. At most four processes run concurrently, and session IDs are private to one task registry. Cancellation, task completion or dropping that registry terminates its processes and descendants. Sessions are pipes, not a PTY, and are not restored across restarts.

## File edits

Use `apply_patch` with one `patch` string delimited by `*** Begin Patch` and `*** End Patch`. File sections use `*** Update File: path`, `*** Add File: path`, or `*** Delete File: path`. Updates use one bare `@@` per hunk followed by lines starting with a space, `-`, or `+`. Include enough exact context for one match. Adds use only `+` lines. Deletes have no body. A separate legacy shape with `path`, `old_text`, and `new_text` still works for exact replacement.

Review the actual diff before approval. Lorca rechecks every target before applying, verifies each write, and attempts to roll back earlier changes if a later one fails. The batch is not a crash-safe filesystem transaction. Moves are not a native patch operation.

## Vision

The normal agent attaches its existing inference session to `view_image`. The tool reads a discovered PNG/JPEG inside the workspace, sends it through that same model's existing image pipeline, and returns a bounded factual description. Image bytes never enter the public activity result or the main tool transcript. Unsupported models report the existing model error; files are limited to 8 MiB, 8192 pixels per side and 25 megapixels. Screenshots must be accessible inside the selected workspace; arbitrary filesystem paths are not implicitly authorized.

## Web availability

`fetch_url` performs real bounded public HTTPS requests and returns source text/HTML. It checks DNS addresses, pins them to the request and checks every redirect; private networks, credentials, proxies and non-443 URLs are excluded. Network calls require approval. It does not execute JavaScript, browse authenticated pages, or control a browser.

General chat `web_search` is registered **only** when `LORCA_WEB_SEARCH_URL` is configured to a valid public HTTPS JSON search endpoint. Its POST request is `{"query":"...","limit":5}`. The endpoint must return `{"results":[{"title":"...","url":"https://...","description":"..."}]}`. Optional `LORCA_WEB_SEARCH_TOKEN` supplies a Bearer token, retained in the handler and never included in schemas or approval text. No search provider or API key is hardcoded. Without configuration, search is explicitly unavailable. Job Apps' existing Exa research adapter is unchanged and is not silently imported into general chat. Ocean is not implemented.

## Add a tool

1. Create its named module in the appropriate drawer and implement `Tool` with a strict schema, preparation/argument checks and a bounded execution result. Reuse `Workspace` and shared transport/process helpers.
2. Register it in that drawer's `register` function with its category and required authority. Preparation must request approval for effects that need consent; a metadata label is not a grant.
3. Add tests for argument failures, scope restrictions, cancellation and real effects. Add factual observation handling only if needed; ordinary tools do not need agent-loop dispatch changes.

The original `Tool` methods and `Registry::add` API remain compatible with existing custom handlers. Non-workspace tools override the optional-workspace methods, allowing discovery and user questions without inventing a filesystem workspace.

## Memory and completion

The Agent drawer also registers `search_history`, `get_history_record`, and
`compact_context`. Recall tools share conversation-scoped SQLite originals and
FTS5. They are read-only, do not grant workspace authority and are discoverable
through the same exact-schema mechanism. Job Apps remains an independent custom
registry without chat memory or input virtualization.

`get_history_record` takes a record ID and either `chunkIndex` or `offset`/`length`,
not both. Offsets count Unicode scalars; `field` defaults to `content`. `metadata`
pages original structured calls and attachment data. `nextOffset`, `totalChars`
and `totalChunks` allow exhaustive source traversal. Recalled excerpts are not
recursively archived and never prove a fresh action.

`update_plan` maintains advisory notes; `compact_context` requests a deterministic
checkpoint, without another inference engine. Model-authored notes are never
execution evidence. A complete, tool-free public answer is the normal stopping
decision. `request_user_input` pauses explicitly; a public clarification question
is also a valid reply. Permission requests never grant authority by themselves.

Malformed top-level arguments receive one exact-schema correction opportunity.
Specific interrupted native-tool HTTP failures get one bounded retry with
small-patch guidance. Length-stopped call batches never execute; native argument
JSON is not stitched together. Repeated identical failures remain blocked.

Public text continuation is separate from native tool arguments. Partial text
segments stay available, are joined without invented whitespace and use
conservative echo suppression. Missing final public text receives one recovery;
repeated non-progress stops explicitly. Hidden reasoning is not resumable storage.

See [architecture](architecture.md) and [validation](v0.5-validation.md) for
resource policies, migration and tested limitations.
