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
        if [',', ';'].contains(&c) {
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

fn directory(clause: &[Token]) -> String {
    let dirs: Vec<_> = clause
        .windows(2)
        .filter_map(|pair| {
            let word = pair[0].text.to_ascii_lowercase();
            let dir = &pair[1].text;
            (matches!(word.as_str(), "in" | "under" | "inside")
                && !matches!(
                    dir.to_ascii_lowercase().as_str(),
                    "this" | "the" | "a" | "an" | "it"
                )
                && std::path::Path::new(dir).extension().is_none())
            .then(|| path_key(dir))
            .flatten()
        })
        .collect();
    if dirs.len() == 1 && !dirs[0].is_empty() {
        dirs[0].clone()
    } else {
        ".".into()
    }
}

pub(super) fn explicit_files(task: &str) -> Vec<String> {
    let mut paths = vec![];
    for clause in clauses(task) {
        let clause = positive(&clause);
        let dir = directory(clause);
        let mut reading = false;
        for (i, token) in clause.iter().enumerate() {
            let lower = token.text.to_ascii_lowercase();
            if !token.quoted && read_verb(&lower) {
                reading = true;
                continue;
            }
            if !token.quoted
                && matches!(
                    lower.as_str(),
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
                )
            {
                reading = false;
            }
            if !reading {
                continue;
            }
            if !token.quoted
                && matches!(
                    lower.as_str(),
                    "in" | "under" | "inside" | "from" | "using" | "based" | "for" | "to"
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
            if !token.quoted && !named && !file_like {
                continue;
            }
            let path = if lower == "readme" {
                "README.md"
            } else {
                &token.text
            };
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
        let words: Vec<_> = clause.iter().map(|t| t.text.to_ascii_lowercase()).collect();
        if !words.iter().any(|w| read_verb(w)) || !words.iter().any(|w| w == "files") {
            continue;
        }
        let text = words.join(" ");
        if !explicit_files(&text).is_empty() {
            continue;
        }
        let all = words
            .windows(2)
            .any(|w| matches!(w[0].as_str(), "all" | "every" | "each") && w[1] == "files")
            || words
                .windows(3)
                .any(|w| w[0] == "read" && w[1] == "the" && w[2] == "files");
        let collection = Collection {
            directory: directory(clause),
            coverage: if all {
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

fn observations(history: &[Value], tool: &str) -> Vec<Value> {
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
fn complete_reads(history: &[Value]) -> Vec<String> {
    let mut reads: BTreeMap<String, (u64, Vec<(u64, u64)>)> = BTreeMap::new();
    for result in observations(history, "read_file") {
        if result["ok"] != true {
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
        let read = reads.entry(path).or_insert_with(|| (total, vec![]));
        if read.0 != total {
            *read = (total, vec![]);
        }
        read.1.push((offset, end));
    }
    reads
        .into_iter()
        .filter_map(|(path, (total, mut intervals))| {
            intervals.sort();
            let mut covered = 0;
            for (start, end) in intervals {
                if start > covered {
                    return None;
                }
                covered = covered.max(end);
            }
            (covered == total).then_some(path)
        })
        .collect()
}

impl Collection {
    pub(super) fn missing(&self, history: &[Value]) -> Vec<String> {
        let listings = observations(history, "list_directory");
        let listing = listings.iter().rev().find(|v| {
            v["ok"] == true && v["path"].as_str().and_then(path_key) == path_key(&self.directory)
        });
        let Some(listing) = listing.filter(|v| v["truncated"] == false && v["entries"].is_array())
        else {
            return vec![format!(
                "List {} completely with list_directory before verifying its file collection",
                self.directory
            )];
        };
        let complete = complete_reads(history);
        let mut missing = vec![];
        let mut members = vec![];
        for item in listing["entries"].as_array().unwrap() {
            if item["kind"] == "directory" {
                continue;
            }
            let path = item["path"].as_str().and_then(path_key);
            if item["kind"] != "file" || path.is_none() {
                missing.push(format!("File collection in {} contains an unreadable or unsupported entry; report its concrete limitation", self.directory));
                continue;
            }
            let path = path.unwrap();
            let parent = std::path::Path::new(&path)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("");
            if path_key(if parent.is_empty() { "." } else { parent }) != path_key(&self.directory) {
                missing.push(format!(
                    "Invalid file collection membership in {}",
                    self.directory
                ));
                continue;
            }
            members.push(path);
        }
        if self.coverage == InspectionCoverage::AllImmediate {
            for path in members {
                if !complete.contains(&path) {
                    missing.push(format!("Read the complete collection member {path} with read_file (continue from nextOffset when needed)"));
                }
            }
        } else if !members.is_empty() && !members.iter().any(|p| complete.contains(p)) {
            missing.push(format!(
                "Read relevant file content from the verified collection in {}",
                self.directory
            ));
        }
        missing
    }
}
