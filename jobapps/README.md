# Job Apps

Native Research, manual Applying, and Email & Follow-ups share the existing local
tracker. Research uses Lorca's Rust agent with a web-only tool registry; no Hermes
runtime, application automation, or email sending is involved.

## Setup

Build with `./macos/build.sh`, quit older Lorca instances, and open the rebuilt app.
Existing records load automatically from `~/Library/Application Support/lorca/job-apps`,
outside Git. On another Mac, connect Google and sync the same Sheet to download saved
job records and shared profile/templates. Google sign-in and the approved PDF remain local.
Review the factual candidate profile and editable research preferences in Settings.
Start pins Lorca's selected local model for the shift. Research continues across tab
changes; Pause it before chatting or switching models. Quit stops research. Job Apps also offers **Unload model…**, using the same
confirmation and active-work safeguards as the chat model menu.

## Google and outreach

In Job Apps Settings → Google, connect the intended Gmail account using
a Google Desktop OAuth client with Gmail API and Sheets API enabled. On the original
Mac the existing Sheets client is reused; a new Mac needs its own Google sign-in and
OAuth client configuration. Credentials, resumes and templates are never committed.

Enter the existing Sheet URL and enable sync. This replaces the legacy launchd
Sheets writer on the original Mac. While Job Apps' companion is running, sync checks
every minute and writes changed job cells only; the Status timestamp refreshes too.
Opening Job Apps or saving profile/preferences/templates requests an immediate sync
when Google is connected and sync is enabled. Requests share the existing serialized
worker and wait for current Google work rather than starting parallel syncs.
Notes, priority, stage and ISO follow-up dates are editable on either side. Conflicts
retain both values for review in Lorca. Evidence and Gmail status come from Lorca.
Stable IDs and job/evidence summaries let a fresh local tracker download Sheet jobs;
full browser logs, the approved resume PDF, credentials and Gmail thread links stay local. Sheets provides no atomic compare-and-swap: a second read checks for changes
before writes, but simultaneous writers are not supported. Run one active Mac.

The **Lorca Settings** tab syncs the candidate profile, search preferences, initial
email template, and follow-up template. Sync first on the Mac with the completed
profile/templates, then sync the other Mac. Default placeholders do not overwrite
saved settings. Different edits are retained for review in **Google**;
choose Keep this Mac or Use Sheet, then sync again. Incoming profile changes wait
until research is paused, including when research starts during an in-flight sync.
Settings errors remain visible without blocking job or Gmail updates. This tab
contains your personal profile, so use the same
private Sheet on both Macs. Google tokens, OAuth clients, API keys, PDF files,
connection/enable state, and local paths are never included in settings sync.

Templates may be left blank until later. Start each with `Subject: ...`, then the
body. Supported placeholders: `{{first_name}}`, `{{contact_name}}`, `{{company}}`,
`{{role}}`, `{{job_url}}`, `{{job_specific}}`, `{{your_name}}`. Personalization uses
stored research facts; template wording is preserved, without another model rewrite.
Local initial drafts are prepared for verified roles with published email evidence.
Select one approved PDF before creating initial Gmail drafts. Review/edit locally,
then choose Approve & send, or Create Gmail draft to finish manually in Gmail.
Sending requires a fresh review of the recipient, message and master PDF; uncertain sends are reconciled rather than retried automatically.

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
field conflicts, Sheet round trips, shared settings/credential exclusion, and record bootstrap. Native SwiftUI acceptance
and live Google draft/Sheets checks were also run on the development Mac.

Settings has separate Search, Profile, Email and Google sections. Text supports
Markdown preview and a plain editor; Preview never alters saved wording. Drop one
PDF into Profile → Resume, or choose it with the file picker, to approve the local
attachment for initial outreach. This does not rewrite the candidate profile.

## Job Apps workflow

Needs you opens actionable outreach: missing recruiter emails (yellow), replies,
follow-ups, and messages awaiting review. Recruiter details supplied in Lorca are
labeled Provided by you. Real conflicting edits remain available for review;
legacy Not publicly established contact labels are treated as empty values.

Outreach attaches the approved master PDF. Approve & send binds approval to the
recipient, text and PDF fingerprint. Ambiguous sends require reconciliation and
are not automatically retried. Existing Gmail drafts are edited/sent in Gmail.
Linked conversations can be refreshed inside Job Apps; replies cancel the due date.

Applying keeps a separate LaTeX copy for each job. Automatic tailoring reorders
existing bullets for relevance without creating new claims. Edit LaTeX supports
manual wording changes; compilation uses Tectonic in untrusted mode. The compiler
is installed into Lorca's local tools folder on first use if needed. The master
source remains separate. Save the PDF, apply on the employer's site, then Mark
applied. Application forms remain manual.

The existing minute-based Sheet worker also transfers local drafts, tracking
references, resume source/selection and the approved master PDF. The PDF is stored
in a hidden Lorca Resume tab in verified chunks and transferred only when its hash
changes. Concurrent master edits require a choice. Credentials stay local. On a
second Mac, connect the same account/Sheet; tailored PDFs can be built from the
synced source. Both Macs need this version before using the new editable fields.
Sheets uses read-before-write conflict checks, not distributed transactions; avoid
simultaneously sending the same message from both Macs. Only one research/model
operation runs on each Mac. Zoom follows the existing chat-text shortcuts and
reflows the Job Apps header without changing navigation elsewhere.
