//! Lightweight hardware hints for installed-model selection. No model is
//! downloaded, reloaded or changed by this module. llama.cpp's measured memory
//! figures are preferred to estimates whenever the backend supplies them.
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub struct HardwareSnapshot {
    pub os: String,
    pub cpu_threads: usize,
    pub physical_memory_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub available_storage_bytes: Option<u64>,
    pub estimated: bool,
}
impl HardwareSnapshot {
    pub fn detect(root: &Path) -> Self {
        let cpu_threads = std::thread::available_parallelism().map_or(1, |v| v.get());
        let (physical, available) = memory_bytes();
        Self {
            os: std::env::consts::OS.into(),
            cpu_threads,
            physical_memory_bytes: physical,
            available_memory_bytes: available,
            available_storage_bytes: available_storage(root),
            estimated: true,
        }
    }
}
#[cfg(target_os="linux")]
fn memory_bytes() -> (Option<u64>,Option<u64>) {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let value = |label: &str| text.lines().find_map(|line| {
        line.strip_prefix(label)?.trim().split_whitespace().next()?
            .parse::<u64>().ok().map(|kib| kib.saturating_mul(1024))
    });
    (value("MemTotal:"),value("MemAvailable:"))
}
#[cfg(target_os="macos")]
fn memory_bytes() -> (Option<u64>,Option<u64>) {
    let sysctl = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n","hw.memsize"]).output().ok();
    let physical = sysctl.as_ref().filter(|x| x.status.success())
        .and_then(|x|std::str::from_utf8(&x.stdout).ok())
        .and_then(|s|s.trim().parse::<u64>().ok());
    // vm_stat's free/inactive/speculative pages are reclaimable estimates, not
    // a guaranteed GPU KV-cache allocation. Leave safety headroom separately.
    let data = std::process::Command::new("/usr/bin/vm_stat").output().ok();
    let available = data.as_ref().filter(|x|x.status.success()).and_then(|x| {
        let output = String::from_utf8_lossy(&x.stdout);
        let page_size = output.lines().next().and_then(|l|l.split("page size of ")
            .nth(1)).and_then(|s|s.split_whitespace().next())
            .and_then(|s|s.parse::<u64>().ok()).unwrap_or(4096);
        let pages = output.lines().filter(|line|[
            "Pages free:","Pages inactive:","Pages speculative:"
        ].iter().any(|label|line.starts_with(label)))
            .filter_map(|line|line.split(':').nth(1)?.trim()
                .trim_end_matches('.').parse::<u64>().ok())
            .sum::<u64>();
        Some(pages.saturating_mul(page_size))
    });
    (physical, available)
}
#[cfg(not(any(target_os="linux",target_os="macos")))]
fn memory_bytes() -> (Option<u64>,Option<u64>) { (None,None) }

#[cfg(unix)]
fn available_storage(root: &Path) -> Option<u64> {
    use std::{ffi::CString,os::unix::ffi::OsStrExt};
    let name = CString::new(root.as_os_str().as_bytes()).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(name.as_ptr(), info.as_mut_ptr()) } != 0 { return None; }
    let info = unsafe { info.assume_init() };
    Some((info.f_bavail as u64).saturating_mul(info.f_frsize as u64))
}
#[cfg(not(unix))]
fn available_storage(_: &Path) -> Option<u64> { None }

pub fn installed_ollama(hardware: &HardwareSnapshot, list: &Value) -> Value {
    let models = list["models"].as_array().cloned().unwrap_or_default();
    let mut results = models.iter().filter_map(|m| {
        let name = m["name"].as_str()?;
        let size = m["size"].as_u64()?;
        // Disk size approximates quantized weights. Runtime overhead, partial
        // offload and KV cache are architecture- and context-dependent.
        let weight_estimate = size.saturating_mul(115) / 100;
        let reserve = 1024 * 1024 * 1024_u64;
        let likely_fit = hardware.available_memory_bytes.map(|free|
            free.saturating_mul(7) / 10 > weight_estimate.saturating_add(reserve));
        Some(json!({
            "model":name,"downloadBytes":size,
            "parameterSize":m["details"]["parameter_size"],
            "quantization":m["details"]["quantization_level"],
            "estimatedWeightMemoryBytes":weight_estimate,
            "likelyFitsWeights":likely_fit,
            "contextFit":"unknown until the backend reports actual KV allocation",
            "confidence":"rough estimate only; not a verified recommendation"
        }))
    }).collect::<Vec<_>>();
    results.sort_by_key(|m|m["downloadBytes"].as_u64().unwrap_or(0));
    json!(results)
}

pub fn installed_llama_cpp(list: &Value) -> Value {
    let mut results = Vec::new();
    for model in list["data"].as_array().into_iter().flatten() {
        if let Some(id) = model["id"].as_str() {
            results.push(json!({
                "model":id,
                "status":model["status"]["value"],
                "minMemory":model["memory"]["min"],
                "maxMemory":model["memory"]["max"],
                "effectiveMemory":model["memory"]["effective"],
                "context":model["meta"]["n_ctx"],
                "confidence":if model["memory"]["status"]=="measured" {
                    "backend measured"
                } else {"unmeasured or unavailable"}
            }));
        }
    }
    json!(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_advice_distinguishes_weight_estimate_from_context_fit() {
        let hardware = HardwareSnapshot {
            os:"test".into(),cpu_threads:8,
            physical_memory_bytes:Some(32_000_000_000),
            available_memory_bytes:Some(24_000_000_000),
            available_storage_bytes:Some(200_000_000_000),estimated:true
        };
        let models=installed_ollama(&hardware,&json!({"models":[
            {"name":"small","size":4_000_000_000_u64,"details":{"parameter_size":"7B","quantization_level":"Q4_K_M"}},
            {"name":"large","size":27_000_000_000_u64,"details":{"parameter_size":"70B","quantization_level":"Q4_K_M"}}
        ]}));
        assert_eq!(models[0]["model"],"small");
        assert_eq!(models[0]["likelyFitsWeights"],true);
        assert_eq!(models[1]["likelyFitsWeights"],false);
        assert_eq!(models[0]["contextFit"],"unknown until the backend reports actual KV allocation");
    }
}
