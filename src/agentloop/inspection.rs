//! Read requirements grounded in user references and task-local tool evidence.
//! Model target labels describe intent; they cannot manufacture filenames.
use super::{intent::InspectionCoverage, recovery::path_key};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Collection {
    pub directory: String,
    pub coverage: InspectionCoverage,
    pub source: &'static str,
    pub unresolved: bool,
}

struct Token {
    text: String,
    quoted: bool,
}

// Quoted paths stay whole. Sentence/clause boundaries and negative instructions
// cannot leak file references into a preceding positive read request.
fn clauses(task: &str) -> Vec<Vec<Token>> {
    let mut clauses = vec![vec![]];
    let mut chars = task.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            if c == '\n' {
                clauses.push(vec![]);
            }
            continue;
        }
        if c == ',' {
            continue;
        }
        if c == ';' {
            clauses.push(vec![]);
            continue;
        }
        let quoted = ['`', '"', '\''].contains(&c);
        let mut text = String::new();
        if quoted {
            for ch in chars.by_ref() {
                if ch == c {
                    break;
                }
                text.push(ch);
            }
        } else {
            text.push(c);
            while chars
                .peek()
                .is_some_and(|c| !c.is_whitespace() && ![',', ';'].contains(c))
            {
                text.push(chars.next().unwrap());
            }
        }
        let boundary = !quoted && text.ends_with('.');
        if !quoted {
            text = text.trim_matches([':', '?', '!']).to_owned();
            if text.len() > 1 && text.ends_with('.') {
                text.pop();
            }
        }
        if !text.is_empty() {
            clauses.last_mut().unwrap().push(Token { text, quoted });
        }
        if boundary {
            clauses.push(vec![]);
        }
    }
    clauses
}

fn positive(clause: &[Token]) -> &[Token] {
    let end = clause
        .iter()
        .enumerate()
        .find_map(|(i, t)| {
            let lower = t.text.to_ascii_lowercase();
            (!t.quoted
                && (matches!(lower.as_str(), "don't" | "dont" | "never" | "without")
                    || (lower == "do"
                        && clause
                            .get(i + 1)
                            .is_some_and(|t| t.text.eq_ignore_ascii_case("not")))))
            .then_some(i)
        })
        .unwrap_or(clause.len());
    &clause[..end]
}

fn read_verb(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "read" | "inspect" | "review" | "examine"
    )
}

fn stop_verb(text: &str) -> bool {
    matches!(
        text.to_ascii_lowercase().as_str(),
        "update"
            | "edit"
            | "modify"
            | "create"
            | "write"
            | "run"
            | "execute"
            | "replace"
            | "fix"
            | "explain"
            | "report"
            | "summarize"
            | "describe"
            | "tell"
            | "list"
            | "give"
            | "preserve"
    )
}

fn directory(clause: &[Token]) -> Option<String> {
    let mut roots = Vec::new();
    for (i, marker) in clause.iter().enumerate().filter(|(_, t)| !t.quoted) {
        if !matches!(
            marker.text.to_ascii_lowercase().as_str(),
            "in" | "under" | "inside" | "beneath"
        ) {
            continue;
        }
        let first = clause.get(i + 1)?;
        let label = first.text.to_ascii_lowercase();
        let target = if !first.quoted && matches!(label.as_str(), "this" | "the") {
            clause.get(i + 2)?
        } else {
            first
        };
        let label = target.text.to_ascii_lowercase();
        let root = if !target.quoted
            && matches!(
                label.as_str(),
                "." | "directory" | "workspace" | "project" | "repo" | "repository"
            ) {
            ".".into()
        } else {
            let root = path_key(&target.text)?;
            if root.is_empty() { ".".into() } else { root }
        };
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    match roots.as_slice() {
        [] => Some(".".into()),
        [root] => Some(root.clone()),
        _ => None,
    }
}

pub(super) fn explicit_files(task: &str) -> Vec<String> {
    let mut paths = vec![];
    let mut reading = false;
    for clause in clauses(task) {
        let clause = positive(&clause);
        let scope_end = clause
            .iter()
            .enumerate()
            .skip_while(|(_, t)| !read_verb(&t.text))
            .find(|(_, t)| !t.quoted && stop_verb(&t.text))
            .map_or(clause.len(), |(i, _)| i);
        let dir = directory(&clause[..scope_end]).unwrap_or_else(|| "..".into());
        // Only a bare coordinated target after “then” inherits a read verb.
        reading = reading
            && clause
                .first()
                .is_some_and(|t| t.text.eq_ignore_ascii_case("then"));
        for (i, token) in clause.iter().enumerate() {
            let lower = token.text.to_ascii_lowercase();
            if !token.quoted && lower == "instead" {
                break;
            }
            if !token.quoted && read_verb(&lower) {
                reading = true;
                continue;
            }
            if !token.quoted && stop_verb(&lower) {
                reading = false;
            }
            if !reading {
                continue;
            }
            if !token.quoted
                && matches!(
                    lower.as_str(),
                    "in" | "under"
                        | "inside"
                        | "beneath"
                        | "from"
                        | "using"
                        | "based"
                        | "for"
                        | "to"
                )
            {
                break;
            }
            let named = i > 0 && clause[i - 1].text.eq_ignore_ascii_case("named");
            let file_like = std::path::Path::new(&token.text)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.chars().any(char::is_alphabetic))
                || matches!(
                    lower.as_str(),
                    "license" | "makefile" | "dockerfile" | "notice" | "copying" | "readme"
                );
            let bare = i > 0
                && read_verb(&clause[i - 1].text)
                && clause.get(i + 1).is_none_or(|next| {
                    matches!(next.text.to_ascii_lowercase().as_str(), "and" | "instead")
                })
                && !matches!(
                    lower.as_str(),
                    "files"
                        | "file"
                        | "everything"
                        | "all"
                        | "it"
                        | "this"
                        | "that"
                        | "repository"
                        | "repo"
                        | "directory"
                );
            if !token.quoted && !named && !file_like && !bare && !token.text.contains('/') {
                continue;
            }
            let path = &token.text;
            if path.ends_with('/') {
                continue;
            }
            let mut path = path_key(path).unwrap_or_else(|| path.to_owned());
            if path.is_empty() {
                continue;
            }
            if !path.contains('/') && dir != "." {
                path = format!("{dir}/{path}");
            }
            if token.text.starts_with("./") && !path.contains('/') {
                path = format!("./{path}");
            }
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn collections(task: &str) -> Vec<Collection> {
    let mut collections = vec![];
    for clause in clauses(task) {
        let clause = positive(&clause);
        let Some(start) = clause
            .iter()
            .position(|token| !token.quoted && read_verb(&token.text))
        else {
            continue;
        };
        let rest = &clause[start..];
        let end = rest
            .iter()
            .skip(1)
            .position(|t| !t.quoted && stop_verb(&t.text))
            .map_or(rest.len(), |i| i + 1);
        let clause = &rest[..end];
        let words: Vec<_> = clause.iter().map(|t| t.text.to_ascii_lowercase()).collect();
        let has = |word: &str| words.iter().any(|w| w == word);
        if !words.iter().any(|w| read_verb(w))
            || !(has("file") || has("files") || has("everything"))
        {
            continue;
        }
        if !explicit_files(
            &clause
                .iter()
                .map(|t| {
                    if t.quoted {
                        format!("`{}`", t.text)
                    } else {
                        t.text.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
        .is_empty()
        {
            continue;
        }
        let exhaustive = ["all", "every", "each", "everything"]
            .iter()
            .any(|w| has(w))
            || words.windows(3).any(|w| w == ["read", "the", "files"]);
        let recursive = ["recursively", "recursive", "nested", "under", "beneath"]
            .iter()
            .any(|w| has(w))
            || (exhaustive && (has("project") || has("repository") || has("repo")));
        let scope = directory(clause);
        let collection = Collection {
            unresolved: scope.is_none(),
            directory: scope.unwrap_or_default(),
            coverage: if exhaustive && recursive {
                InspectionCoverage::AllRecursive
            } else if exhaustive {
                InspectionCoverage::AllImmediate
            } else {
                InspectionCoverage::Relevant
            },
            source: "user_instruction",
        };
        if !collections.contains(&collection) {
            collections.push(collection);
        }
    }
    collections
}

pub(super) fn observations(history: &[Value], tool: &str) -> Vec<Value> {
    history
        .iter()
        .filter(|m| m["role"] == "tool" && m["tool_name"] == tool)
        .filter_map(|m| serde_json::from_str::<Value>(m["content"].as_str()?).ok())
        .map(|v| {
            if v["alreadyExecuted"] == true {
                v["evidence"].clone()
            } else {
                v
            }
        })
        .collect()
}

// Full coverage requires actual UTF-8 byte intervals, including byte zero.
// The last excerpt alone, duplicate chunks and gaps cannot certify a full read.
pub(super) fn complete_reads(history: &[Value]) -> Vec<String> {
    covered_reads(history, None)
}

pub(super) fn covered_reads(history: &[Value], range: Option<(u64, u64)>) -> Vec<String> {
    let mut reads = BTreeMap::<String, ReadCoverage>::new();
    for result in observations(history, "read_file") {
        if result["ok"] != true || !result["error"].is_null() {
            continue;
        }
        let Some(path) = result["path"].as_str().and_then(path_key) else {
            continue;
        };
        let (Some(offset), Some(total), Some(content)) = (
            result["offset"].as_u64(),
            result["totalBytes"].as_u64(),
            result["content"].as_str(),
        ) else {
            continue;
        };
        let Some(end) = offset
            .checked_add(content.len() as u64)
            .filter(|e| *e <= total)
        else {
            continue;
        };
        if result["truncated"].as_bool() != Some(end < total) {
            continue;
        }
        let version = result["version"].clone();
        let read = reads.entry(path).or_default();
        if version.is_null() || read.version != version || read.total != total {
            *read = ReadCoverage {
                total,
                version,
                intervals: vec![],
            };
        }
        read.intervals.push((offset, end));
    }
    reads
        .into_iter()
        .filter_map(|(path, mut read)| {
            read.intervals.sort();
            let (start, end) = range.unwrap_or((0, read.total));
            if end > read.total {
                return None;
            }
            let mut covered = start;
            for (start, end) in read.intervals {
                if start > covered {
                    break;
                }
                covered = covered.max(end);
            }
            (covered >= end).then_some(path)
        })
        .collect()
}

#[derive(Default)]
struct ReadCoverage {
    total: u64,
    version: Value,
    intervals: Vec<(u64, u64)>,
}

// Production evidence comes from the task transcript or its linked continuation.
// Revalidate its workspace and file generation without rereading content.
pub(super) fn current_observations(
    history: Vec<Value>,
    workspace: Option<&super::Workspace>,
) -> Vec<Value> {
    let mut calls = BTreeMap::new();
    history
        .into_iter()
        .filter(|message| {
            if message["role"] == "assistant" {
                if let Some(group) = message["tool_calls"].as_array() {
                    for call in group {
                        if let Some(id) = call["id"].as_str() {
                            calls.insert(id.to_owned(), call["function"].clone());
                        }
                    }
                }
                return true;
            }
            let Some(tool @ ("read_file" | "list_directory")) = message["tool_name"].as_str()
            else {
                return true;
            };
            if message["role"] != "tool" {
                return false;
            }
            let Some(call) = message["tool_call_id"]
                .as_str()
                .and_then(|id| calls.remove(id))
            else {
                return false;
            };
            let Ok(result) =
                serde_json::from_str::<Value>(message["content"].as_str().unwrap_or(""))
            else {
                return false;
            };
            let requested = call["arguments"]["path"].as_str().and_then(path_key);
            let observed = result["path"].as_str().and_then(path_key);
            if call["name"] != tool {
                return false;
            }
            if result["ok"] != true {
                return call["arguments"]["path"] == result["path"]
                    || (requested.is_some() && requested == observed);
            }
            if requested.is_none() || requested != observed {
                return false;
            }
            workspace.is_some_and(|w| w.matches_observation(tool, &result))
        })
        .collect()
}

impl Collection {
    fn latest_listing(&self, history: &[Value]) -> Option<Value> {
        observations(history, "list_directory")
            .into_iter()
            .rev()
            .find(|v| {
                v["ok"] == true
                    && v["error"].is_null()
                    && v["path"].as_str().and_then(path_key) == path_key(&self.directory)
            })
    }

    pub(super) fn lists_member(&self, history: &[Value], path: &str) -> bool {
        if self.unresolved {
            return false;
        }
        let Some(path) = path_key(path) else {
            return false;
        };
        let parent = path.rsplit_once('/').map_or(".", |(parent, _)| parent);
        let mut dir = parent;
        let root = path_key(&self.directory).unwrap_or_default();
        {
            let scoped = Collection {
                directory: dir.into(),
                ..self.clone()
            };
            let Some(list) = scoped.latest_listing(history) else {
                return false;
            };
            let child = &path;
            if !list["entries"].as_array().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["kind"] == "file"
                        && entry["path"].as_str().and_then(path_key).as_ref() == Some(child)
                })
            }) {
                return false;
            }
            if path_key(dir).as_deref() == Some(root.as_str()) {
                return true;
            }
            if self.coverage != InspectionCoverage::AllRecursive {
                return false;
            }
            // Confirm the ancestor chain is actually in the recursive collection.
            while dir != "." {
                let ancestor = dir.rsplit_once('/').map_or(".", |(ancestor, _)| ancestor);
                let scoped = Collection {
                    directory: ancestor.into(),
                    ..self.clone()
                };
                let Some(list) = scoped.latest_listing(history) else {
                    return false;
                };
                if !list["entries"].as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["kind"] == "directory"
                            && entry["path"].as_str().and_then(path_key).as_deref() == Some(dir)
                    })
                }) {
                    return false;
                }
                if path_key(ancestor).as_deref() == Some(root.as_str()) {
                    return true;
                }
                dir = ancestor;
            }
            false
        }
    }

    pub(super) fn missing(&self, history: &[Value]) -> Vec<String> {
        if self.unresolved {
            return vec![
                "The requested collection scope is unresolved; specify one authorized directory"
                    .into(),
            ];
        }
        let complete = complete_reads(history);
        let mut missing = vec![];
        let mut members = vec![];
        let mut directories = vec![(self.directory.clone(), 0)];
        let mut scanned = 0;
        while let Some((directory, depth)) = directories.pop() {
            if depth > 16 {
                return vec!["Exhaustive inspection exceeds the supported directory depth".into()];
            }
            let scoped = Collection {
                directory,
                ..self.clone()
            };
            let listing = scoped.latest_listing(history);
            let Some(listing) =
                listing.filter(|v| v["truncated"] == false && v["entries"].is_array())
            else {
                let detail = if scoped
                    .latest_listing(history)
                    .is_some_and(|v| v["truncated"] == true)
                {
                    "listing exceeds the bounded 200-entry limit"
                } else {
                    "complete list_directory evidence required"
                };
                missing.push(format!(
                    "List {} completely with list_directory ({detail})",
                    scoped.directory
                ));
                continue;
            };
            for item in listing["entries"].as_array().unwrap() {
                scanned += 1;
                if scanned > 2000 {
                    return vec!["Exhaustive inspection exceeds the supported directory depth or entry budget".into()];
                }
                let kind = item["kind"].as_str().unwrap_or("");
                if kind == "directory" && self.coverage != InspectionCoverage::AllRecursive {
                    continue;
                }
                let Some(path) = item["path"].as_str().and_then(path_key) else {
                    missing.push(format!(
                        "File collection in {} contains an unreadable or unsupported entry",
                        scoped.directory
                    ));
                    continue;
                };
                let parent = path.rsplit_once('/').map_or(".", |(parent, _)| parent);
                if path.is_empty() || path_key(parent) != path_key(&scoped.directory) {
                    missing.push(format!(
                        "Invalid file collection membership in {}",
                        scoped.directory
                    ));
                    continue;
                }
                match kind {
                    "directory" => directories.push((path, depth + 1)),
                    "file" => members.push(path),
                    _ => missing.push(format!("Unsupported collection entry {path}")),
                }
            }
        }
        if self.coverage == InspectionCoverage::Relevant {
            if !members.is_empty() && !members.iter().any(|p| complete.contains(p)) {
                missing.push(format!(
                    "Read relevant file content from the verified collection in {}",
                    self.directory
                ));
            }
        } else {
            for path in members {
                if !complete.contains(&path) {
                    missing.push(format!("Read the complete collection member {path} with read_file (continue from nextOffset when needed)"));
                }
            }
        }
        missing
    }
}
