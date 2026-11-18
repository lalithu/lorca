# Embedded model notes

Each note keeps one native text document and undo history, including offscreen.
The model follows independent passages on the existing local model session.
Moving to another passage starts settled work without waiting for all typing to
stop. Edits elsewhere and leaving Notes do not cancel it. Editing its own input,
pausing, changing models, or foreground model work cancels that request.

The app owns source anchors and fences responses by request, note, passage and
model. Unrelated passages are excluded from each task. Completed work is inserted
as full, editable text or a compact native table, with a faint blue background.
There is no response card or expand/apply step. Contributions use the same text
storage, caret, selection and undo system as typing. Generated ranges are tracked
separately so model output does not recursively prompt itself. Undo is respected;
untouched adjacent contributions can refresh, while edited content is preserved.
A bounded completion history prevents immediate reinsertion after undo.

Search uses the existing bundled Exa transport: a bounded query, up to five
sources, one concise result. The backend attaches actual source links. Retrieved
text is evidence, never tool authority. No messaging, booking or arbitrary shell
actions are exposed to the model.

Image paste reuses the existing image importer. Optimized files live separately
from the small note/RTF document, so typing does not rewrite image bytes. Native
OCR supplies readable text; a vision-capable selected model also receives the
actual image. Text-only models can summarize readable flyers, but cannot describe
unseen photo details. Captions and flyer breakdowns become editable note text.

For event flyers, the model selects from dates detected by macOS, requiring an
explicit year and time. It cannot supply an invented calendar timestamp. A valid
event adds a normal inline Add to Calendar link and the parsed local time. The
link opens a generated .ics file in the system calendar importer; no calendar
account is read or silently written. Missing details remain visible. Calendar
files use stable image-based UIDs, UTC times, escaped fields and UTF-8 line folding.

Typography reuses Lorca's Iowan/serif, system sans and monospaced palette. The
Model following footer and quiet margin cue remain. Work continues while the app
runs, rather than through an external daemon after quitting. Oversized passages
and ambiguous duplicate anchors are left alone.

Checks use disposable data roots and the normal Swift/macOS SDK and Rust build.
Focused tests cover editable insertion, undo across notes, tables, image restore,
calendar escaping/link activation, and cancellation. Opt-in live tests exercise
vision captions and the OCR fallback with installed local models. Acceptance
never edits saved personal notes or imports events into a real calendar.
