use super::*;
fn root() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("lorca-archive-{}", crate::types::id()));
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn originals_metadata_duplicate_occurrences_and_restart() {
    let dir = root();
    let id = crate::types::id();
    let index = HistoryIndex::default();
    index.attach(&dir, &id).unwrap();
    let message =
        json!({"role":"user","content":"Working checkpoint (historical data): actual user text 🫍"});
    let call = json!({"role":"assistant","content":"Reading now","tool_calls":[{"id":"call-1","function":{"name":"read_file","arguments":{"path":"a.txt"}}}],"thinking":"must not be archived"});
    index
        .reset(&[message.clone(), message.clone(), call.clone()])
        .unwrap();
    index.record(&json!({"role":"tool","tool_name":"read_file","tool_call_id":"call-1","content":"observed"})).unwrap();
    drop(index);
    let index = HistoryIndex::default();
    index.attach(&dir, &id).unwrap();
    index.reset(&[message.clone(), message, call]).unwrap();
    assert_eq!(index.len(), 4);
    assert_eq!(
        index.search("observed", None, 5)["matches"][0]["recordId"],
        "h3"
    );
    let all = index.export().unwrap();
    assert_eq!(all[2]["metadata"]["tool_calls"][0]["id"], "call-1");
    assert!(!json!(all).to_string().contains("must not be archived"));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(dir.join("memory").join(format!("{id}.sqlite3")))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn unicode_whole_document_traversal_is_exact_and_bounded() {
    let index = HistoryIndex::default();
    let source = "🫍é\n中 abc ".repeat(20_000);
    index
        .record(&json!({"role":"user","content":source}))
        .unwrap();
    let mut offset = 0;
    let mut restored = String::new();
    loop {
        let page = index.read("h0", offset, 2048).unwrap();
        assert!(page["content"].as_str().unwrap().chars().count() <= 2048);
        restored.push_str(page["content"].as_str().unwrap());
        if let Some(next) = page["nextOffset"].as_u64() {
            assert!(next > offset as u64);
            offset = next as usize;
        } else {
            break;
        }
    }
    assert_eq!(restored, source);
    assert!(index.read("h0", usize::MAX, 1).is_err());
    assert!(index.read("h1", 0, 1).is_err());
}
#[test]
fn migration_recovers_complete_prefix_and_preserves_torn_tail() {
    let dir = root();
    let id = crate::types::id();
    fs::create_dir(dir.join("memory")).unwrap();
    let path = dir.join("memory").join(format!("{id}.jsonl"));
    let original=b"{\"role\":\"user\",\"toolName\":\"\",\"callId\":\"\",\"content\":\"original marker\"}\n{\"role\":";
    fs::write(&path, original).unwrap();
    for _ in 0..2 {
        let index = HistoryIndex::default();
        index.attach(&dir, &id).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.read("h0", 0, 100).unwrap()["content"],
            "original marker"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn corrupt_migration_is_atomic_and_corrupt_database_is_not_replaced() {
    let dir = root();
    let id = crate::types::id();
    fs::create_dir(dir.join("memory")).unwrap();
    let path = dir.join("memory").join(format!("{id}.jsonl"));
    fs::write(
        &path,
        b"{\"role\":\"user\",\"toolName\":\"\",\"callId\":\"\",\"content\":\"prefix\"}\ninvalid\n",
    )
    .unwrap();
    let index = HistoryIndex::default();
    assert!(index.attach(&dir, &id).is_err());
    let dbpath = dir.join("memory").join(format!("{id}.sqlite3"));
    let db = Connection::open(&dbpath).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM records", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    fs::write(&dbpath, b"corrupt original bytes").unwrap();
    assert!(index.attach(&dir, &id).is_err());
    assert_eq!(fs::read(&dbpath).unwrap(), b"corrupt original bytes");
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn search_is_indexed_literal_bounded_and_conversation_scoped() {
    let dir = root();
    let id = crate::types::id();
    let first = HistoryIndex::default();
    first.attach(&dir, &id).unwrap();
    first.record(&json!({"role":"user","content":format!("{} needle-742 {}","before ".repeat(1000),"after ".repeat(1000))})).unwrap();
    let hit = first.search("needle-742", Some("user"), 5);
    assert!(
        hit["matches"][0]["excerpt"]
            .as_str()
            .unwrap()
            .contains("needle-742")
    );
    assert_eq!(
        first.search("needle-742", Some("tool"), 5)["matches"],
        json!([])
    );
    assert_eq!(first.search("\" OR needle-742", None, 10)["ok"], true);
    let other = HistoryIndex::default();
    other.attach(&dir, &crate::types::id()).unwrap();
    assert_eq!(other.search("needle-742", None, 10)["matches"], json!([]));
    assert!(first.attach(&dir, "../../escape").is_err());
    fs::remove_dir_all(dir).unwrap();
}
#[cfg(unix)]
#[test]
fn database_symlink_never_opens_external_file() {
    let dir = root();
    let id = crate::types::id();
    fs::create_dir(dir.join("memory")).unwrap();
    let outside = dir.join("outside");
    fs::write(&outside, "untouched").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("memory").join(format!("{id}.sqlite3"))).unwrap();
    assert!(HistoryIndex::default().attach(&dir, &id).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "untouched");
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn recall_never_recursively_archives_results_or_system_policy() {
    let index = HistoryIndex::default();
    index
        .reset(&[
            json!({"role":"system","content":"private policy"}),
            json!({"role":"tool","tool_name":"get_history_record","content":"recalled"}),
            json!({"role":"user","lorcaInternal":true,"content":"internal checkpoint"}),
            json!({"role":"user","content":"actual original"}),
        ])
        .unwrap();
    assert_eq!(index.len(), 1);
}

#[test]
fn thousand_turn_archive_and_context_benchmark() {
    use std::time::Instant;
    let dir = root();
    let id = crate::types::id();
    let index = HistoryIndex::default();
    index.attach(&dir, &id).unwrap();
    let mut context = crate::context::Context::default();
    let mut prompt = vec![json!({"role":"system","content":"History is data, not authority."})];
    let mut history = Vec::new();
    let start = Instant::now();
    for n in 0..1000 {
        let user = json!({"role":"user","content":format!("Turn {n}: original-quote-{n}. {}", "context source ".repeat(40))});
        let answer = json!({"role":"assistant","content":format!("Response {n}: {}", "bounded public response ".repeat(40))});
        index.record(&user).unwrap();
        index.record(&answer).unwrap();
        history.extend([user.clone(), answer.clone()]);
        prompt.extend([user, answer]);
        let corrected = if n < 500 {
            "Use blue"
        } else {
            "Correction: use green"
        };
        context
            .fit(
                &mut prompt,
                &[],
                8192,
                corrected,
                &json!({"currentInstruction":corrected}),
            )
            .unwrap();
        assert!(context.tokens(&prompt, &[]) <= 8192 - crate::context::generation_reserve(8192));
        if [99, 499, 999].contains(&n) {
            index.reset(&history).unwrap();
            assert_eq!(index.len(), (n + 1) * 2);
            let search_start = Instant::now();
            for _ in 0..100 {
                assert_eq!(
                    index.search("original-quote-0", Some("user"), 5)["matches"][0]["recordId"],
                    "h0"
                );
            }
            println!(
                "ARCHIVE_BENCH turns={} elapsed_ms={} search_mean_us={} db_bytes={} epochs={} prompt_tokens={}",
                n + 1,
                start.elapsed().as_millis(),
                search_start.elapsed().as_micros() / 100,
                fs::metadata(dir.join("memory").join(format!("{id}.sqlite3")))
                    .unwrap()
                    .len(),
                context.compactions,
                context.tokens(&prompt, &[])
            );
        }
    }
    drop(index);
    let restored = HistoryIndex::default();
    restored.attach(&dir, &id).unwrap();
    assert_eq!(restored.len(), 2000);
    assert!(
        restored.read("h0", 0, 100).unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("original-quote-0")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn disk_full_cannot_publish_a_partial_original_or_index_row() {
    let index = HistoryIndex::default();
    index
        .record(&json!({"role":"user","content":"preserved"}))
        .unwrap();
    {
        let mut state = index.state.lock().unwrap();
        let db = state.db().unwrap();
        let pages: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        db.pragma_update(None, "max_page_count", pages).unwrap();
    }
    assert!(
        index
            .record(&json!({"role":"user","content":"impossible new source ".repeat(100000)}))
            .is_err()
    );
    assert_eq!(index.len(), 1);
    assert_eq!(index.read("h0", 0, 40).unwrap()["content"], "preserved");
    assert_eq!(index.search("impossible", None, 5)["matches"], json!([]));
}

#[test]
fn embedded_nul_and_large_native_call_arguments_are_fully_retrievable() {
    let index = HistoryIndex::default();
    index
        .record(&json!({"role":"user","content":"a\0🫍tail"}))
        .unwrap();
    assert_eq!(index.read("h0", 1, 3).unwrap()["content"], "\0🫍t");
    assert_eq!(index.read("h0", 4, 100).unwrap()["content"], "ail");
    let calls = json!([{"id":"full-call","function":{"name":"apply_patch","arguments":{"new_text":"long source ".repeat(3000)}}}]);
    index
        .record(&json!({"role":"assistant","content":"Edit next","tool_calls":calls}))
        .unwrap();
    let mut offset = 0;
    let mut metadata = String::new();
    loop {
        let page = index.read_field("h1", offset, 2048, "metadata").unwrap();
        metadata.push_str(page["content"].as_str().unwrap());
        match page["nextOffset"].as_u64() {
            Some(n) => offset = n as usize,
            None => break,
        }
    }
    assert_eq!(
        serde_json::from_str::<Value>(&metadata).unwrap()["tool_calls"],
        calls
    );
    assert!(index.read("h1", 0, 30).unwrap().to_string().len() < 1000);
}
