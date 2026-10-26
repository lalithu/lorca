# Job Apps

Native Research, manual Applying, and Email & Follow-ups share the existing local
tracker. Research uses Lorca's Rust agent with a web-only tool registry; no Hermes
runtime, application automation, or email sending is involved.

## Setup

Build with `./macos/build.sh`, quit older Lorca instances, and open the rebuilt app.
Import the existing `m2max-job-pipeline` folder once on this Mac. SQLite backup copies
records/history, profile, packets and research evidence; the original stays intact.
Runtime data lives in `~/Library/Application Support/lorca/job-apps`, outside Git.
Review the factual candidate profile and editable research preferences in Settings.
Start pins Lorca's selected local model for the shift. Research continues across tab
changes; Pause it before chatting or switching models. Quit stops research.

## Google and outreach

In Job Apps Settings → Google & templates, connect the intended Gmail account using
a Google Desktop OAuth client with Gmail API and Sheets API enabled. On the original
Mac the existing Sheets client is reused; a new Mac needs its own Google sign-in and
OAuth client configuration. Credentials, resumes and templates are never committed.

Enter the existing Sheet URL and enable sync. This replaces the legacy launchd
Sheets writer on the original Mac. While Job Apps' companion is running, sync checks
every minute and writes changed job cells only; the Status timestamp refreshes too.
Notes, priority, stage and ISO follow-up dates are editable on either side. Conflicts
retain both values for review in Lorca. Evidence and Gmail status come from Lorca.
Stable IDs and job/evidence summaries let a fresh local tracker download Sheet jobs;
full browser logs, candidate profile, templates, resume and Gmail thread links are
local. Sheets provides no atomic compare-and-swap: a second read checks for changes
before writes, but simultaneous writers are not supported. Run one active Mac.

Templates may be left blank until later. Start each with `Subject: ...`, then the
body. Supported placeholders: `{{first_name}}`, `{{contact_name}}`, `{{company}}`,
`{{role}}`, `{{job_url}}`, `{{job_specific}}`, `{{your_name}}`. Personalization uses
stored research facts; template wording is preserved, without another model rewrite.
Local initial drafts are prepared for verified roles with published email evidence.
Select one approved PDF before creating initial Gmail drafts. Review/edit locally,
then choose Create Gmail draft. Lorca never sends: send manually in Gmail.

Only linked drafts, resulting threads and Lorca's unique message IDs are checked.
A missing draft is not treated as sent. Provider-confirmed sends schedule one
follow-up after five weekdays in Eastern time (public holidays are not excluded).
Use Follow-up due to find those jobs, prepare one follow-up, and send it manually.
Replies clear the due date and mark Needs review. Uncertain draft creation is held
for reconciliation instead of retried, preventing duplicate outreach. Arbitrary
inbox search, autonomous sending and application automation are unavailable.

## Research and implementation

SwiftUI → private serialized stdio Python companion → SQLite/evidence/checkpoints.
A bundled `lorca-core job-session` uses the existing Rust Agent and Ollama Inference.
Only web_search and web_extract are exposed, with discovery limited to two searches;
research sessions allow five searches and seven extracts. Extracted official-source
quotes, not search snippets, establish verification. Exa free transport is rate
limited; errors are visible and bounded retries/checkpoints retain progress.

The companion uses Python's standard library; Python 3 must be installed. Exa
transport is adapted from Hermes' MIT implementation; attribution is in vendor/.

## Verification

```sh
python3 -B -m unittest discover -s jobapps/tests
cargo test
cargo clippy --all-targets -- -D warnings
./macos/build.sh
```

Tests cover source validation, research transport/lifecycle, template preservation,
PDF approval, duplicate-draft prevention, provider-confirmed send/reply tracking,
field conflicts, Sheet round trips and record bootstrap. Native SwiftUI acceptance
and live Google draft/Sheets checks were also run on the development Mac.
