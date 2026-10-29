use super::*;
pub(super) fn read_length() -> usize {
    8192
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PathArgs {
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadArgs {
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "read_length")]
    pub length: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SearchArgs {
    pub query: String,
    #[serde(default = "dot")]
    pub path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
}
pub(super) fn dot() -> String {
    ".".into()
}
pub(super) fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {e}"))
}
pub(super) fn string(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("Missing {key}"))
}
