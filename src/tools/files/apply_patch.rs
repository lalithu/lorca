use crate::tools::{arguments::*, *};
use std::collections::HashSet;

pub struct ApplyPatch;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    patch: String,
}

#[derive(serde::Serialize, Deserialize)]
struct Change {
    path: String,
    expected: Option<String>,
    replacement: Option<String>,
}

impl Change {
    fn kind(&self) -> &'static str {
        match (&self.expected, &self.replacement) {
            (None, _) => "Created",
            (_, None) => "Deleted",
            _ => "Modified",
        }
    }
}

impl Tool for ApplyPatch {
    fn schema(&self) -> Value {
        let mut s = schema(
            "apply_patch",
            "Read the current file before proposing a small edit. Requires approval. Patch sections: Add File, Update File, Delete File. @@ starts update hunks only. Add File takes + lines, or no body for an empty file. Alternatively use one exact replacement.",
            json!({"patch":{"type":"string"},"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}}),
            vec![],
        );
        s["function"]["parameters"]["oneOf"] = json!([
            {"required":["patch"]},
            {"required":["path","old_text","new_text"]}
        ]);
        s["function"]["parameters"]["examples"] = json!([
            {"path":"README.md","old_text":"","new_text":""},
            {"patch":"*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch"}
        ]);
        s
    }

    fn prepare(&self, w: &Workspace, arguments: Value, _limits: &Limits) -> Result<Prepared> {
        check_size(&arguments)?;
        let (changes, diff) = if arguments.get("patch").is_some() {
            let input: Patch = parse(arguments)?;
            prepare_patch(w, &input.patch)?
        } else {
            let a: PatchArgs = parse(arguments)?;
            let path = w.relative(&a.path)?;
            let old = source(w, &path)?;
            let new = match &old {
                None if a.old_text.is_empty() => a.new_text,
                Some(s) if unique_text(s, &a.old_text) => {
                    s.replacen(&a.old_text, &a.new_text, 1)
                }
                _ => return Err("old_text must match exactly once; empty old_text is only for creating a new file".into()),
            };
            let diff = full_diff(&path, old.as_deref(), Some(&new));
            (
                vec![Change {
                    path,
                    expected: old,
                    replacement: Some(new),
                }],
                diff,
            )
        };
        for change in &changes {
            if change.expected == change.replacement
                || change
                    .replacement
                    .as_ref()
                    .is_some_and(|s| s.len() > super::super::workspace::FILE_LIMIT)
            {
                return Err(format!("Patch for {} is empty or too large", change.path));
            }
        }
        let paths: Vec<_> = changes.iter().map(|c| &c.path).collect();
        let mut action = json!({"paths":paths,"diff":diff});
        let mut payload = json!({"changes":changes});
        if let [change] = changes.as_slice() {
            action["path"] = json!(change.path);
            action["changeKind"] = json!(change.kind());
            payload["path"] = json!(change.path);
            payload["replacement"] = json!(change.replacement);
        }
        Ok(Prepared {
            action,
            payload,
            approval: true,
        })
    }

    fn execute<'a>(
        &'a self,
        w: &'a Workspace,
        p: &'a Prepared,
        _limits: &'a Limits,
        cancel: watch::Receiver<bool>,
        _output: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            let changes: Vec<Change> = parse(p.payload["changes"].clone())?;
            // Recheck the entire batch after approval, before the first mutation.
            let permissions = changes
                .iter()
                .map(|change| w.patch_permissions(&change.path, &change.expected))
                .collect::<Result<Vec<_>>>()?;

            for (index, change) in changes.iter().enumerate() {
                let result = if *cancel.borrow() {
                    Err("Cancelled".into())
                } else {
                    w.write_patch(
                        &change.path,
                        &change.expected,
                        change.replacement.as_deref(),
                        permissions[index].clone(),
                    )
                };
                if let Err(error) = result {
                    let mut failures = Vec::new();
                    for (applied, mode) in changes[..index].iter().zip(&permissions).rev() {
                        if let Err(rollback) = w.write_patch(
                            &applied.path,
                            &applied.replacement,
                            applied.expected.as_deref(),
                            mode.clone(),
                        ) {
                            failures.push(format!("{}: {rollback}", applied.path));
                        }
                    }
                    return Err(if failures.is_empty() {
                        format!("{}: {error}", change.path)
                    } else {
                        format!(
                            "{}: {error}. Rollback failed for {}",
                            change.path,
                            failures.join(", ")
                        )
                    });
                }
            }
            let results: Vec<_> = changes.iter().map(|change| {
                if change.replacement.is_some() {
                    let _ = w.remember(&change.path);
                }
                json!({"path":change.path,"changeKind":change.kind(),"bytes":change.replacement.as_ref().map(String::len)})
            }).collect();
            let mut result = json!({"ok":true,"verified":true,"changes":results});
            if let [change] = results.as_slice() {
                result["path"] = change["path"].clone();
                result["bytes"] = change["bytes"].clone();
            }
            Ok(result)
        })
    }
}

fn unique_text(text: &str, needle: &str) -> bool {
    needle
        .chars()
        .next()
        .and_then(|first| {
            text.find(needle)
                .map(|offset| !text[offset + first.len_utf8()..].contains(needle))
        })
        .unwrap_or(false)
}

fn source(w: &Workspace, path: &str) -> Result<Option<String>> {
    let old = w.patch_source(path)?;
    if old.is_some() {
        w.require_reference(path)?;
    } else if let Some(parent) = std::path::Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        w.require_reference(&parent.to_string_lossy())?;
    }
    Ok(old)
}

fn prepare_patch(w: &Workspace, patch: &str) -> Result<(Vec<Change>, String)> {
    let mut lines = patch.lines().peekable();
    if lines.next() != Some("*** Begin Patch") {
        return Err("Invalid arguments: patch must start with *** Begin Patch and end with *** End Patch. Use the displayed format.".into());
    }
    let mut changes = Vec::new();
    let mut diff = String::new();
    let mut seen = HashSet::new();
    loop {
        let header = lines.next().ok_or("Patch must end with *** End Patch")?;
        if header == "*** End Patch" {
            if lines.next().is_some() || changes.is_empty() {
                return Err(
                    "Expected one or more file sections and no text after *** End Patch".into(),
                );
            }
            return Ok((changes, diff));
        }
        let (kind, path) = header
            .split_once(": ")
            .ok_or("Expected an Add File, Update File or Delete File header")?;
        let path = w.relative(path)?;
        if !seen.insert(path.clone()) {
            return Err(format!("Duplicate file section: {path}"));
        }
        if changes.len() == 32 {
            return Err("Split patches spanning more than 32 files".into());
        }
        let old = source(w, &path)?;
        let mut body = Vec::new();
        while lines.peek().is_some_and(|line| !line.starts_with("*** ")) {
            body.push(lines.next().unwrap());
        }
        let new = match (kind, &old) {
            ("*** Add File", None) => {
                let mut new = String::new();
                for line in body {
                    new.push_str(
                        line.strip_prefix('+')
                            .ok_or("Invalid arguments: Add File uses + lines only, with no @@. For an empty file omit the entire body, including any + line.")?,
                    );
                    new.push('\n');
                }
                diff.push_str(&full_diff(&path, None, Some(&new)));
                Some(new)
            }
            ("*** Delete File", Some(old)) if body.is_empty() => {
                diff.push_str(&full_diff(&path, Some(old), None));
                None
            }
            ("*** Update File", Some(old)) => {
                diff.push_str(&format!("--- {path}\n+++ {path}\n"));
                Some(update(old, &body, &mut diff).map_err(|err| format!("{path}: {err}"))?)
            }
            _ => {
                return Err(format!(
                    "Invalid file section or wrong file existence: {header}"
                ));
            }
        };
        changes.push(Change {
            path,
            expected: old,
            replacement: new,
        });
    }
}

fn update(old: &str, body: &[&str], diff: &mut String) -> Result<String> {
    let source: Vec<_> = old.split_inclusive('\n').collect();
    let ending = if old.contains("\r\n") { "\r\n" } else { "\n" };
    let mut cursor = 0;
    let mut new = String::new();
    let mut new_line = 0;
    if body.first() != Some(&"@@") {
        return Err("Each update hunk must start with a bare @@ line".into());
    }
    let mut hunks = body[1..].split(|line| *line == "@@").peekable();
    let mut hunk_index = 0;
    while let Some(hunk) = hunks.next() {
        hunk_index += 1;
        let mut context = Vec::new();
        let mut additions = 0;
        let mut removals = 0;
        for line in hunk {
            match line.as_bytes().first() {
                Some(b'+') => additions += 1,
                Some(b'-') => {
                    removals += 1;
                    context.push(&line[1..]);
                }
                Some(b' ') => context.push(&line[1..]),
                _ => return Err("Prefix every hunk line with space, - or +".into()),
            }
        }
        if (context.is_empty() && !(source.is_empty() && hunk_index == 1 && hunks.peek().is_none()))
            || additions + removals == 0
        {
            return Err(
                "Include exact surrounding context or removed lines and at least one change".into(),
            );
        }
        let offset = if context.is_empty() {
            0
        } else {
            let mut locations =
                source[cursor..]
                    .windows(context.len())
                    .enumerate()
                    .filter(|(_, lines)| {
                        lines
                            .iter()
                            .map(|line| {
                                line.strip_suffix("\r\n")
                                    .or_else(|| line.strip_suffix('\n'))
                                    .unwrap_or(line)
                            })
                            .eq(context.iter().copied())
                    });
            let (offset, _) = locations
                .next()
                .ok_or("Hunk context is stale; read the file and propose a fresh patch")?;
            if locations.next().is_some() {
                return Err("Hunk context is ambiguous; include more surrounding lines".into());
            }
            offset
        };
        let start = cursor + offset;
        for line in &source[cursor..start] {
            new.push_str(line);
        }
        new_line += offset;
        let new_count = context.len() - removals + additions;
        let mut preview = String::new();
        cursor = start;
        for line in hunk {
            match line.as_bytes()[0] {
                b'+' => {
                    if !new.is_empty() && !new.ends_with('\n') {
                        return Err(
                            "To append after an unterminated line, replace that line in the hunk"
                                .into(),
                        );
                    }
                    new.push_str(&line[1..]);
                    new.push_str(ending);
                    diff_line(&mut preview, '+', &format!("{}{ending}", &line[1..]));
                }
                marker => {
                    let original = source[cursor];
                    cursor += 1;
                    if marker == b' ' {
                        new.push_str(original);
                    }
                    diff_line(&mut preview, marker as char, original);
                }
            }
        }
        let new_start = new_line + usize::from(new_count != 0);
        diff.push_str(&format!(
            "@@ -{},{} +{new_start},{new_count} @@\n",
            start + usize::from(!context.is_empty()),
            context.len()
        ));
        diff.push_str(&preview);
        new_line += new_count;
    }
    for line in &source[cursor..] {
        new.push_str(line);
    }
    Ok(new)
}

fn diff_line(diff: &mut String, prefix: char, line: &str) {
    diff.push(prefix);
    diff.push_str(line);
    if !line.ends_with('\n') {
        diff.push_str("\n\\ No newline at end of file\n");
    }
}

fn full_diff(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    let header = |text: Option<&str>| if text.is_some() { path } else { "/dev/null" };
    let count = |text: Option<&str>| text.unwrap_or_default().lines().count();
    let (old_count, new_count) = (count(old), count(new));
    let mut diff = format!(
        "--- {}\n+++ {}\n@@ -{},{old_count} +{},{new_count} @@\n",
        header(old),
        header(new),
        usize::from(old_count > 0),
        usize::from(new_count > 0)
    );
    for (prefix, text) in [('-', old), ('+', new)] {
        for line in text.unwrap_or_default().split_inclusive('\n') {
            diff_line(&mut diff, prefix, line);
        }
    }
    diff
}
