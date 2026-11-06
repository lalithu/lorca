//! Shared Hugging Face GGUF catalog helpers. No downloads or provider side effects.
//! Recommendations are hardware-fit estimates, not benchmark-quality rankings.
use serde_json::{Value, json};
use std::collections::BTreeMap;

const QUANTS: &[(&str, u8)] = &[
    ("Q8_0", 90), ("Q6_K", 80), ("Q5_K_M", 75), ("Q5_K_S", 70),
    ("Q4_K_M", 65), ("Q4_K_S", 60), ("IQ4_XS", 58),
    ("Q4_0", 55), ("MXFP4", 54), ("Q3_K_M", 40), ("Q3_K_S", 35),
    ("Q2_K", 20),
];

/// Never let a remote model identifier escape the expected Hugging Face
/// namespace/repository grammar. This is also used before local download/delete.
pub fn valid_repo(name: &str) -> bool {
    let mut parts = name.split('/');
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    parts.next().is_none()
        && !first.is_empty() && !second.is_empty()
        && name.len() <= 200
        && [first, second].iter().all(|p| {
            !p.starts_with('.') && !p.ends_with('.')
                && p.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
        })
}
pub fn valid_runtime_model(name: &str) -> bool {
    !name.is_empty() && name.len() <= 300 && !name.contains("..")
        && name.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'/' | b'.' | b'_' | b'-' | b':')
        })
}
pub fn valid_quant(name: &str) -> bool {
    name.len() <= 32 && !name.is_empty() && name.bytes().all(|c| {
        c.is_ascii_alphanumeric() || c == b'_'
    })
}
pub fn runtime_id(repo: &str, quant: &str, ollama: bool) -> Option<String> {
    (valid_repo(repo) && valid_quant(quant)).then(|| {
        if ollama { format!("hf.co/{repo}:{quant}") } else { format!("{repo}:{quant}") }
    })
}

/// Conservative starting budget for a single Metal-loaded model. Real free
/// memory and model-specific KV/compute measurements must still be checked.
pub fn model_memory_budget(physical: u64, metal_working_set: Option<u64>) -> u64 {
    const GIB: u64 = 1024 * 1024 * 1024;
    let working = metal_working_set.unwrap_or(physical.saturating_mul(3) / 4);
    working.saturating_sub(GIB).min(physical.saturating_sub(4 * GIB))
}

fn quant_for_filename(filename: &str) -> Option<(&'static str, u8)> {
    let name = filename.to_ascii_uppercase();
    if !name.ends_with(".GGUF") || ["MMPROJ", "MM_PROJ", "VISION", "DRAFT", "MTP", "EAGLE"]
        .iter().any(|v| name.contains(v)) { return None; }
    QUANTS.iter().copied().find(|(q, _)| {
        name.contains(&format!("-{q}"))
            || name.contains(&format!("_{q}"))
            || name.contains(&format!(".{q}"))
            || name.contains(&format!("/{q}/"))
    })
}
fn parameter_hint(id: &str) -> Option<String> {
    id.split(['/', '-', '_']).find(|word| {
        let w = word.to_ascii_uppercase();
        let n = w.trim_end_matches('B').trim_end_matches('M');
        (w.ends_with('B') || w.ends_with('M')) && n.parse::<f32>().is_ok()
    }).map(str::to_owned)
}

pub fn candidate(summary: &Value, details: &Value, memory_budget: u64) -> Value {
    let repo = details["id"].as_str().or_else(||summary["id"].as_str()).unwrap_or("");
    let mut grouped: BTreeMap<&str, (u64, u8, usize)> = BTreeMap::new();
    let mut vision = false;
    for file in details["siblings"].as_array().into_iter().flatten() {
        let Some(name) = file["rfilename"].as_str() else {continue};
        if name.to_ascii_lowercase().contains("mmproj") { vision = true; }
        if let Some((quant,quality)) = quant_for_filename(name) {
            let entry = grouped.entry(quant).or_insert((0,quality,0));
            let bytes = file["size"].as_u64().unwrap_or(0);
            entry.0 = entry.0.saturating_add(bytes);
            entry.2 += 1;
        }
    }
    let mut builds: Vec<_> = grouped.into_iter().map(|(quant,(bytes,quality,count))| {
        json!({"quant":quant,"sizeBytes":if bytes>0 {Some(bytes)} else {None},
            "qualityOrder":quality,"fileCount":count})
    }).collect();
    builds.sort_by(|a,b|b["qualityOrder"].as_u64().cmp(&a["qualityOrder"].as_u64()));
    // Only claim a specific quant fits if its download sizes were confirmed.
    // Leave 1.5 GiB additional headroom for KV and compute as a provisional
    // measure; actual fitting remains a backend-specific operation.
    const KV_ALLOWANCE: u64 = 1536 * 1024 * 1024;
    let selected = builds.iter().find(|b| {
        b["sizeBytes"].as_u64().is_some_and(|size|
            size.saturating_mul(105)/100 + KV_ALLOWANCE <= memory_budget)
    });
    let recommended = selected.and_then(|b|b["quant"].as_str()).map(str::to_owned);
    let download_bytes = selected.and_then(|b|b["sizeBytes"].as_u64());
    json!({
        "repo":repo,
        "name":repo.split('/').next_back().unwrap_or(repo),
        "parameters":summary["safetensors"]["total"]
            .as_u64().map(|n|format!("{:.1}B",n as f64 / 1e9))
            .or_else(||parameter_hint(repo)),
        "downloads":summary["downloads"].as_u64().unwrap_or(0),
        "likes":summary["likes"].as_u64().unwrap_or(0),
        "vision":vision || summary["tags"].as_array().is_some_and(|tags|
            tags.iter().any(|t|t.as_str().is_some_and(|s|s.contains("image-text-to-text")))),
        "recommendedQuant":recommended,
        "sizeBytes":download_bytes,
        "builds":builds,
        "estimatedFit":recommended.is_some(),
        "fitStatus":if recommended.is_some() {"weight estimate only"} else {"unknown or exceeds conservative memory budget"},
        "source":"Hugging Face",
        "url":if valid_repo(repo) {format!("https://huggingface.co/{repo}")} else {String::new()},
        "contextFit":"unverified until runtime allocation and KV measurements"
    })
}
pub fn summary_candidate(summary: &Value) -> Value {
    let repo = summary["id"].as_str().unwrap_or("");
    json!({"repo":repo,"name":repo.split('/').next_back().unwrap_or(repo),
        "parameters":parameter_hint(repo),"downloads":summary["downloads"].as_u64().unwrap_or(0),
        "likes":summary["likes"].as_u64().unwrap_or(0),
        "vision":false,"recommendedQuant":null,"sizeBytes":null,
        "estimatedFit":false,"builds":[],
        "fitStatus":"Inspect GGUF file sizes to determine compatibility",
        "source":"Hugging Face",
        "url":if valid_repo(repo){format!("https://huggingface.co/{repo}")}else{String::new()},
        "contextFit":"unverified"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_or_malformed_repo_and_quant_strings() {
        assert!(valid_repo("unsloth/Qwen3.5-9B-GGUF"));
        assert!(!valid_repo("evil/../../model"));
        assert!(!valid_repo("evil\nhost/repo"));
        assert!(!valid_quant("Q4_K_M\n"));
        assert_eq!(runtime_id("unsloth/test-GGUF","Q4_K_M",true).unwrap(),
            "hf.co/unsloth/test-GGUF:Q4_K_M");
    }
    #[test]
    fn picks_highest_precision_known_quant_that_fits() {
        let g = 1024 * 1024 * 1024_u64;
        let info=json!({"id":"maker/Foo-9B-GGUF","siblings":[
            {"rfilename":"foo-Q8_0.gguf","size":13*g},
            {"rfilename":"foo-Q5_K_M.gguf","size":8*g},
            {"rfilename":"foo-Q4_K_M.gguf","size":6*g},
            {"rfilename":"mmproj-f16.gguf","size":100}
        ]});
        let c=candidate(&json!({"id":"maker/Foo-9B-GGUF"}), &info,10*g);
        assert_eq!(c["recommendedQuant"],"Q5_K_M");
        assert_eq!(c["vision"],true);
        assert_eq!(c["sizeBytes"],8*g);
        let unknown=candidate(&json!({"id":"maker/Foo-9B-GGUF"}), &json!({}),10*g);
        assert_eq!(unknown["recommendedQuant"],Value::Null);
    }
}
