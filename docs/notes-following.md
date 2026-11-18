# Notes: embedded model following

Notes uses one small background queue on the existing model session. Passages
are separated by blank lines or new headings; their meaning is interpreted by
the model, without a command vocabulary. A completed passage becomes eligible
when the caret leaves it. The passage being edited gets a short local delay.
Editing elsewhere or leaving Notes does not cancel its work. Editing that
passage, pausing, changing models, or foreground model work cancels its request.
Only passages visited in Notes are scheduled, and one inference runs at a time.

Each request carries the exact passage and a bounded surrounding-context
snapshot. Results are fenced by request, note, source and model identity. The
backend still owns model preparation, foreground priority and cancellation.
This is document-aware scheduling, not continuous model perception or new model
weights/memory. Large passages over 12 KB and ambiguous duplicate anchors are
left alone rather than edited unsafely.

The model may propose a table, structure, priorities, a breakdown, or one web
search. Search reuses the bundled `jobapps/web.py` Exa transport with a bounded
query and five results. Only the search query goes to Exa. Retrieved text is
untrusted evidence, never tool authority. The follow-up response is constrained
to one concise annotation; actual source links are attached by the backend.
Failures stay visible beside the passage and require a retry or source edit.
There are no email, booking, arbitrary shell or other external-action tools.

Quiet inline activity and result excerpts are native document decorations.
They do not become authored text, move focus, or enter undo history. Existing
margin cues and the Model following menu remain. Results persist separately
with the note and are blue until opened. Opening a result offers the existing
review/apply flow; applying remains one undoable document edit. Work continues
across tabs while the app runs; this is not a daemon that works after quitting.

Acceptance uses disposable data roots and a separately identified Lorca Preview
bundle built from the same Swift/Rust binaries, SDK, target and resources as the
main app. Tests never edit saved personal notes.
