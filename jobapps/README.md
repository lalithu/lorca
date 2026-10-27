# 🫍 Job Apps

A guided native workflow for research, manual applications, and recruiter outreach.
SwiftUI talks to a private serialized Python companion; SQLite stores local progress.
Research uses lorca's existing Rust runtime with web-only tools.

## Setup

1. Build with `./macos/build.sh`, quit the old app, and reopen it.
2. In **Job Apps → Settings**, review Search preferences and your factual Candidate profile.
3. Import an approved master résumé PDF in Profile; optionally supply its LaTeX source for tailoring.
4. For Gmail/Sheets, configure a Google Desktop OAuth client with Gmail and Sheets APIs enabled, then connect Google and enter a private Sheet URL.

Python 3 must be installed. This personal build currently targets the author's Gmail account; it is not a general multi-account product. Credentials and private runtime data live outside Git in `~/Library/Application Support/lorca/job-apps/`.

## The daily flow

- **Research:** start explicitly with the selected model. Pause before chatting or switching models. Research keeps progress across tabs and stops on quit.
- **Applying:** open the posting, review/tailor the résumé, apply on the employer's site, then mark Applied.
- **Recruiter:** add published contact evidence or enter a contact yourself, labeled Provided by you.
- **Review in Gmail:** Prepare outreach fills the saved template, attaches the approved master PDF, and opens the draft. Review and send manually in Gmail.
- **Waiting:** a confirmed send turns the job green, removes it from the actionable queue, and schedules follow-up five weekdays later. No manual date entry is needed.

**Needs you** focuses on the oldest remaining discovery number and keeps the current job visible until **Next job**. **Skip for now** defers it for the current pass. **All jobs** retains newest-first order. Due follow-ups and unreviewed replies return to the queue; **Done reviewing reply** records a synced receipt until a newer reply arrives.

Notes are always visible and editable in one live Markdown surface. Save also requests sync. Priority and application stage remain editable.

## Templates, drafts, and tracking

- Default subject: `Role @ Company`. An optional `Subject:` line in the template overrides it.
- Supported fields: `{{first_name}}`, `{{contact_name}}`, `{{company}}`, `{{role}}`, `{{job_url}}`, `{{job_specific}}`, `{{your_name}}`.
- Common bracket fields such as `[NAME]`, `[ROLE]`, `[COMPANY]`, skills, and an experience sentence fill from saved facts without loading a model. Unknown fields or missing facts stop preparation.
- `[ATTACH RESUME]` is replaced by the actual attachment, not left in the email body.
- Repeated preparation reuses the linked draft. **Fill template & open Gmail** repairs an old placeholder draft while preserving attachments; it refuses to overwrite a body already edited in Gmail.
- Lorca checks linked drafts/threads for provider-confirmed sends and replies. A missing draft alone is never evidence of sending.
- Confirmed replies clear the due date and mark Needs review. Follow-up uses five weekdays in Eastern time, excluding weekends but not public holidays.
- Uncertain Gmail writes require reconciliation; writes are never replayed automatically. Interrupted Google reads retry once. TLS verification remains enabled.
- No automatic email sending or application submission. The normal workflow ends at Gmail review/send; any retained backend send operation still requires explicit approval.

## Two-Mac sync

Connect both Macs to the same private Sheet and enable sync. Opening Job Apps, returning to the app, or saving settings requests sync; the running companion also checks roughly every minute. Requests use one serialized worker.

| Synced through Sheets | Stays on each Mac |
| --- | --- |
| Job records, stable IDs, notes, priority, stage, contacts | Google sign-in, OAuth client, tokens, API keys |
| Draft/tracking references and follow-up dates | Connection/enable state and local paths |
| Candidate profile, preferences, email templates | Full research/browser logs and local caches |
| Master LaTeX source and approved master PDF | Chat conversations and attachments |

- **Lorca Settings** holds shared text; hidden **Lorca Resume** holds the approved PDF in verified chunks. PDF transfer is hash-based, with a 15 MB limit.
- Import/replace the PDF inside lorca; changing an external original file does not update its approved copy. PDF import is picked up by the next background/manual sync. Let sync finish before closing.
- PDF, LaTeX source, and candidate profile are separate. Changing one does not rewrite the others. Tailored PDFs can be rebuilt from synced job source.
- Conflicts retain both values for review; defaults do not overwrite completed settings. Choose Keep this Mac or Use Sheet, then sync again.
- Sheets uses read-before-write checks, not atomic distributed transactions. Use one active Mac at a time, especially for sending or conflicting résumé edits.
- After pulling code updates, rebuild and restart both Macs. Git carries code; Sheets carries Job Apps data. Each Mac signs in separately.

## Résumé tailoring

Applying keeps a separate LaTeX copy per job. Automatic tailoring reorders existing bullets for relevance without inventing claims; manual LaTeX edits allow wording changes. Tectonic compiles in untrusted mode and is installed into lorca's local tools folder on first use if needed. The master source stays separate. Application forms remain manual.

## Research limits

- Exa search/extraction transport is adapted from Hermes' MIT implementation; no Hermes runtime is bundled. See [attribution](vendor/NOTICE.md).
- Discovery allows two searches; research sessions allow five searches and seven extracts. Official-source quotes establish verification; snippets are leads only.
- Searches/extraction use external services. Errors are bounded and checkpoints retain progress; missing or unverifiable facts stay unknown.
- One research/model operation per Mac. Research does not control a browser, apply for jobs, or send outreach.

## Verification

```sh
python3 -B -m unittest discover -s jobapps/tests
cargo test
cargo clippy --all-targets -- -D warnings
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test --package-path macos
./macos/build.sh
```

Tests cover research validation, templates, PDF approval, shared settings/PDF transfer, two-Mac round trips, schema compatibility, conflicts, duplicate-draft protection, and confirmed send/reply tracking. Native workflow, Gmail draft repair/send tracking, and live Sheets sync were verified on the development Mac. This checkpoint still needs a physical second-Mac acceptance check.
