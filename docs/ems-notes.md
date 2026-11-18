# Ems Notes frontend

Embedded Model Interfaces (Ems) bring assistance into the work itself. This iteration explores a quiet native interaction before model following is connected.

## Try it

Write `Table with data:` and some lines below it. Pause for 1.2 seconds. A small table glyph appears in the document's left margin. Click it to inspect the native popover, then choose **Use table** or dismiss it. Continuing to type closes the preview. The footer menu can pause and resume suggestions.

This is a local UI preview. It does not call Qwen or another model. Structured pipe/tab rows can become columns; unstructured lines are preserved in a single Details column. Semantic interpretation and inferred column names belong to the later model integration.

## Behavior

- No suggestion panel or automatic popover while writing; only a small margin cue after a pause.
- No document mutation until Use table. The replacement is scoped to the cue and its data, stopping at the following blank line.
- Editing, switching notes, or pausing cancels pending suggestions. The preview checks its source before applying a change.
- Tables are native editable text tables; insertion is one undoable action. Cursor location and the editor's scroll position are retained where possible.
- Notes with tables store an RTF representation alongside their plain text. Existing plain notes remain readable. Ordinary restyling preserves table blocks.
- No image search, generation, downloads, or model activity is started by this UI.

## Validation

Focused native tests cover cue boundaries, missing values, preservation of unstructured text, stale-source rejection, native table RTF round trips, undo, persistent notes, caret restoration, corrupt-file preservation, and delayed margin-cue/pause behavior. Build and restart use the UI-only workflow and reuse the existing backend.
