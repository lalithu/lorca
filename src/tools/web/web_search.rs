use super::http;
use crate::tools::{arguments::parse, *};
pub struct WebSearch {
    pub(super) endpoint: String,
    pub(super) token: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    query: String,
    #[serde(default = "limit")]
    limit: usize,
}
fn limit() -> usize {
    5
}
impl Tool for WebSearch {
    fn schema(&self) -> Value {
        schema(
            "web_search",
            "Search the configured public HTTPS JSON search integration. Returns actual provider results, never invented results. Requires approval.",
            json!({"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":5}}),
            vec!["query"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let args: Args = parse(a.clone())?;
        if args.query.trim().is_empty() || args.query.len() > 1500 || !(1..=5).contains(&args.limit)
        {
            return Err("Query must contain 1–1500 bytes; limit 1–5".into());
        }
        let host = http::url(&self.endpoint)?
            .host_str()
            .unwrap_or_default()
            .to_owned();
        Ok(Prepared {
            action: json!({"query":args.query,"limit":args.limit,"scope":format!("Send this query to {host} over HTTPS.")}),
            payload: json!({"query":args.query,"limit":args.limit}),
            approval: true,
        })
    }
    fn execute<'a>(
        &'a self,
        _: &'a Workspace,
        p: &'a Prepared,
        l: &'a Limits,
        c: watch::Receiver<bool>,
        o: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        self.execute_optional(None, p, l, c, o)
    }
    fn execute_optional<'a>(
        &'a self,
        _: Option<&'a Workspace>,
        p: &'a Prepared,
        _: &'a Limits,
        c: watch::Receiver<bool>,
        _: &'a mut (dyn FnMut(Value) + Send),
    ) -> FutureResult<'a, Value> {
        Box::pin(async move {
            let args: Args = parse(p.payload.clone())?;
            let (_, _, text, truncated) = http::request(
                &self.endpoint,
                Some(json!({"query":args.query,"limit":args.limit})),
                self.token.as_deref(),
                c,
            )
            .await?;
            if truncated {
                return Err("Search provider response exceeded limit".into());
            }
            search_results(&text, args.limit)
        })
    }
}

fn search_results(text: &str, limit: usize) -> Result<Value> {
    let data: Value =
        serde_json::from_str(text).map_err(|_| "Search endpoint returned invalid JSON")?;
    let entries = data["results"]
        .as_array()
        .ok_or("Search endpoint must return {results:[{title,url,description}]} JSON")?;
    let results: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let url = entry["url"].as_str()?;
            http::url(url).ok()?;
            Some(json!({
                "url":url,
                "title":bounded(entry["title"].as_str().unwrap_or_default(),300),
                "description":bounded(entry["description"].as_str().unwrap_or_default(),1000)
            }))
        })
        .take(limit)
        .collect();
    if !entries.is_empty() && results.is_empty() {
        return Err("Search endpoint returned no usable public HTTPS result URLs".into());
    }
    Ok(json!({"ok":true,"results":results,"untrustedSource":true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_bounded_and_only_real_provider_urls_are_returned() {
        let response = r#"{"results":[{"url":"https://127.0.0.1/private"},{"url":"https://example.org/article","title":"Article","description":"An actual response entry"},{"url":"https://example.net/other"}]}"#;
        let value = search_results(response, 1).unwrap();
        assert_eq!(
            value["results"],
            json!([{"url":"https://example.org/article","title":"Article","description":"An actual response entry"}])
        );
        assert_eq!(value["untrustedSource"], true);
        assert!(search_results("not JSON", 5).is_err());
        assert!(search_results(r#"{"results":[{"url":"http://localhost/"}]}"#, 5).is_err());
        assert_eq!(
            search_results(r#"{"results":[]}"#, 5).unwrap()["results"],
            json!([])
        );
    }

    #[test]
    fn search_approval_shows_query_and_host_but_never_credentials() {
        let tool = WebSearch {
            endpoint: "https://example.org/search".into(),
            token: Some("private-token".into()),
        };
        let prepared = tool
            .prepare_optional(None, json!({"query":"search terms"}), &Limits::default())
            .unwrap();
        assert!(prepared.approval);
        assert_eq!(prepared.action["query"], "search terms");
        assert_eq!(prepared.payload["limit"], 5);
        assert!(
            prepared.action["scope"]
                .as_str()
                .unwrap()
                .contains("example.org")
        );
        assert!(!prepared.action.to_string().contains("private-token"));
    }
}
