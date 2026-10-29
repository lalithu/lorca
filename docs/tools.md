# Tool chest

`src/tools/mod.rs` defines the small `Tool` interface and prepared action. `registry.rs` owns the handlers, schemas, descriptions, drawer categories and required authority. Each drawer registers its own working handlers:

| Drawer | Modules |
| --- | --- |
| Files | `read_file`, `list_directory`, `search_files`, `apply_patch`, `git_diff` |
| Terminal | `exec_command`, `write_stdin`; shared sandbox/process implementation in `command.rs`, task-local sessions in `sessions.rs` |
| Vision | `view_image` |
| Web | `fetch_url`, `web_search`; shared bounded public-HTTPS transport in `http.rs` |
| Agent | `tool_search`, `update_plan`, `request_user_input`, `request_permissions` |

## Discovery and authority

The normal registry starts with directory listing, file reading, file search, `apply_patch`, `request_user_input`, both terminal schemas (`exec_command` and `write_stdin`), and `tool_search`. Terminal argument types and a JSON example are available before the first command. Invalid calls return their expected schema and a concise correction hint; corrected retries retain the existing approval gateway, while identical failed calls stay blocked. Search matches registered names, descriptions and drawer names without an extra model call or task classifier, and makes up to five matching schemas available on subsequent turns. Hidden tools remain subject to the same authorization gateway; visibility is not permission. The full schema API and explicitly restricted `Registry::empty()` registries retain their existing behavior, including Job Apps' independent web-only tools. No capabilities are injected into those registries.

All calls still go through `agentloop/authority.rs`: task restrictions, preparation, single-use human approval when required, cancellation, execution, and real result evidence. Files retain capability-based workspace boundaries, discovered references and atomic patch verification. Plans do not count as completion evidence. Questions pause as an incomplete task in the existing chat; the next user message provides the answer. Permission requests likewise pause and never grant access, change a workspace, or relax the command policy.

## Terminal

`exec_command` accepts the same `executable`, `arguments`, and optional `directory` as the old `run_command`, plus optional `yield_time_ms` (0–1000, default 1000). **The existing approved Cargo allowlist and macOS OS sandbox are unchanged.** This is not an unrestricted shell. Legacy `run_command` retains its original wait-until-complete schema and behavior.

A running command returns `sessionId`. Poll with `write_stdin` using `session_id` and optional `yield_time_ms`. Supply `chars` (up to 4096 bytes) and/or `close_stdin` to interact; these writes require a fresh approval, while empty polling does not. Output is bounded to 16 KiB per command, with a real exit code when finished. The original command timeout remains in force between polls. At most four processes run concurrently, and session IDs are private to one task registry. Cancellation, task completion or dropping that registry terminates its processes and descendants. Sessions are pipes, not a PTY, and are not restored across restarts.

## Vision

The normal agent attaches its existing inference session to `view_image`. The tool reads a discovered PNG/JPEG inside the workspace, sends it through that same model's existing image pipeline, and returns a bounded factual description. Image bytes never enter the public activity result or the main tool transcript. Unsupported models report the existing model error; files are limited to 8 MiB, 8192 pixels per side and 25 megapixels. Screenshots must be accessible inside the selected workspace; arbitrary filesystem paths are not implicitly authorized.

## Web availability

`fetch_url` performs real bounded public HTTPS requests and returns source text/HTML. It checks DNS addresses, pins them to the request and checks every redirect; private networks, credentials, proxies and non-443 URLs are excluded. Network calls require approval. It does not execute JavaScript, browse authenticated pages, or control a browser.

General chat `web_search` is registered **only** when `LORCA_WEB_SEARCH_URL` is configured to a valid public HTTPS JSON search endpoint. Its POST request is `{"query":"...","limit":5}`. The endpoint must return `{"results":[{"title":"...","url":"https://...","description":"..."}]}`. Optional `LORCA_WEB_SEARCH_TOKEN` supplies a Bearer token, retained in the handler and never included in schemas or approval text. No search provider or API key is hardcoded. Without configuration, search is explicitly unavailable. Job Apps' existing Exa research adapter is unchanged and is not silently imported into general chat. Ocean is not implemented.

## Add a tool

1. Create its named module in the appropriate drawer and implement `Tool` with a strict schema, preparation/argument checks and a bounded execution result. Reuse `Workspace` and shared transport/process helpers.
2. Register it in that drawer's `register` function with its category and required authority. Preparation must request approval for effects that need consent; a metadata label is not a grant.
3. Add tests for argument failures, scope restrictions, cancellation and real effects. Add verification evidence handling only if the tool introduces a new kind of verifiable outcome; ordinary tools do not need agent-loop dispatch changes.

The original `Tool` methods and `Registry::add` API remain compatible with existing custom handlers. Non-workspace tools override the optional-workspace methods, allowing discovery and user questions without inventing a filesystem workspace.

## Agent guidance and completion

One concise system policy identifies the model as the agent inside Lorca. Rust keeps the immutable objective, authority, original transcript and verified execution evidence; the model chooses capabilities and interprets observations. The policy and unavailable-tool facts survive compaction without being promoted from untrusted history. Current native Ollama function schemas are sent in `tools`; no unsupported JSON-format/tool-call combination or alternate model API is introduced.

Malformed structured argument objects receive strict schema feedback; invalid top-level tool JSON gets one model correction before a real guidance question. No malformed call executes. Unchanged failed calls remain blocked, retaining the original error. Short standalone model clarification questions route through the existing registered `request_user_input` handler instead of provoking repeated completion prompts. Repeated failures, unchanged observations or exhausted continuation budgets pause for free-text guidance. Custom registries are never expanded to add a missing question tool.

Exact unverified read paths use the existing registered listing/search and read handlers internally, through the same authority gateway. A path is recovered once per task; the task iteration budget also bounds the number of different recovered paths. Filesystem capability checks, symlink protections and approval on custom discovery handlers remain enforced. A missing/prohibited path cannot become a successful read through narrative or discovery alone.

Atomic patch readback supplies verified path and byte counts. Explicit single-target empty-file constraints are checked before approval and again by completion verification (zero bytes). This is a focused deterministic check, not a general natural-language outcome solver; other requested quantities, locations and semantics remain model decisions constrained by schemas, workspace permissions, real results and the original objective.

Opt-in real-model smoke checks: `LORCA_LIVE_MODEL=qwen3.5:9b-q4_K_M cargo test --test agent_live -- --ignored --nocapture --test-threads=1`. These use disposable workspaces, approve only the exact empty README, and unload the test model if it was initially unloaded. The normal test suite uses scripted models and real tool execution, plus mock HTTP Ollama protocol tests; it does not silently run live inference.

## Generation budgets and output exhaustion

`context/budget.rs` computes one per-call budget from the actual allocated context window: initial output is one eighth of the window, capped at 8,192 tokens; recovery can double it up to 16,384. A separate safety margin and the output allowance are reserved together during context fitting. The exact same budget reaches the Ollama adapter (including the native core and the restricted research wrapper); it does not select another model or enlarge the allocated context. Existing non-agent context callers retain their legacy reserve behavior.

Ollama `done_reason` is retained as a typed stop/length/unknown outcome. Length-stopped turns discard every pending call before argument parsing or approval. Older responses without a reason remain supported; if their generated count reaches the requested cap, the adapter conservatively reports exhaustion. Actual reported input must also fit the reserved budget before any tool proposal is accepted.

The agent audits `generation_truncated` and permits one consecutive recovery, retaining verified work and the active request. Recovery can use a larger allowance and smaller targeted patches. If the larger allowance cannot fit essential input, failed compaction bookkeeping is rolled back and recovery uses the original smaller-step budget with that input intact; configured timeouts and the task iteration limit remain enforced. Repeated exhaustion reports a specific generation-limit blocker without invoking the generic user-question or ordinary no-action continuation path. Real progress permits a later recovery without creating an unlimited retry loop. Malformed calls, connection interruptions, cancellation and timeouts retain their existing separate failure handling. A truncated image-description result is not reported as a complete successful inspection.

The confirmed allocated window remains the resource ceiling; this does not resize the model or add a hardware auto-tuner. Job Apps retains its independent registry and behavior.

## Working checkpoints and collaboration

`update_plan` accepts optional `checkpoint` notes: `approach`, `decisions`, `uncertainties`, and `nextAction`. Each complete validated update replaces the current notes. Compaction carries them separately from verified operation state. Notes, completed plan steps, and user choices never prove execution or authorize a tool.

`request_user_input` accepts a question, optional recommendation, and up to four options. A real question ends inference in `waiting_for_input`; answering in the same chat/workspace links the saved objective and actual transcript records. The UI's Continue task action explicitly resumes an interrupted request. Ordinary requests after a completed or interrupted task start a new objective. Linked records survive restart; no old approval token or running process is revived.

Chats offers persisted Off / Nominal / High effort, fixed per submission. Native thinking controls come from Ollama's advertised values, not model names or template guesses. Unknown support retains the runtime default. Planning-only turns are bounded (one/two/three between useful actions); ordinary answers do not require a planning cycle. The output/time budget uses confirmed context capacity and sane observed throughput, with a conservative initial estimate and finite time ceilings. A length stop or the specifically recognized malformed-tool HTTP 500 triggers the existing bounded, smaller-patch recovery; no incomplete arguments execute. A checkpoint cannot be recovered from private or half-generated reasoning.
