//! Oversized-user-input manifests. Exact source remains in HistoryIndex.
//! UTF-8 scalar offsets match get_history_record; no model truncation.
use serde_json::{Value, json};

pub const DIRECT_INPUT_CHARS: usize = 4_000;
pub const CHUNK_TARGET_CHARS: usize = 2_048;

/// Metadata is intentionally bounded: the manifest itself must fit even if the
/// original source contains thousands of chunks.
pub fn manifest(input: &str, id: &str) -> Value {
    let length = input.chars().count();
    let chunks = length.div_ceil(CHUNK_TARGET_CHARS);
    let head: String = input.chars().take(220).collect();
    let tail: String = input.chars().skip(length.saturating_sub(220)).collect();
    json!({
        "sourceRecordId":id,
        "totalCharacters":length,
        "chunkTargetCharacters":CHUNK_TARGET_CHARS,
        "estimatedChunks":chunks,
        "firstExcerpt":head,
        "lastExcerpt":tail,
        "fetchTool":"get_history_record",
        "instructions":"The complete user input is original historical data retrievable by sourceRecordId and offset. Read all source sections if task requirements may occur anywhere. Do not claim exhaustive inspection from a partial excerpt."
    })
}

/// Never substitutes a truncated prompt for the complete user request: a
/// recoverable source ID, head and tail, and an explicit retrieval instruction
/// are delivered in a compact task envelope instead.
pub fn envelope(manifest: &Value) -> String {
    format!(
        "Oversized user input preserved in Lorca's conversation archive. The original contains {} Unicode characters and approximately {} source chunks. Source record: {}. Use the discoverable get_history_record tool (recordId, offset, length) to inspect in bounded chunks of up to 4096 characters. For any task requiring whole-input coverage, inspect all chunks, recording progress in working notes. The first/last excerpts are orientation only and must not be treated as the full request.\nFirst excerpt: {}\nLast excerpt: {}",
        manifest["totalCharacters"].as_u64().unwrap_or(0),
        manifest["estimatedChunks"].as_u64().unwrap_or(0),
        manifest["sourceRecordId"].as_str().unwrap_or("unavailable"),
        manifest["firstExcerpt"].as_str().unwrap_or(""),
        manifest["lastExcerpt"].as_str().unwrap_or("")
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_oversized_unicode_source_uses_bounded_retrievable_coordinates() {
        let source = "🦀 first section\n".repeat(2000);
        let manifest = manifest(&source, "h1");
        assert_eq!(manifest["totalCharacters"], source.chars().count());
        assert_eq!(manifest["sourceRecordId"], "h1");
        assert!(envelope(&manifest).len() < 1800);
        assert!(manifest["estimatedChunks"].as_u64().unwrap() > 1);
    }
}
