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
        prepared(a, true)
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
            let data: Value =
                serde_json::from_str(&text).map_err(|_| "Search endpoint returned invalid JSON")?;
            let entries = data["results"]
                .as_array()
                .ok_or("Search endpoint must return {results:[{title,url,description}]} JSON")?;
            let results:Vec<_>=entries.iter().take(args.limit).filter_map(|v|{let url=v["url"].as_str()?;http::url(url).ok()?;Some(json!({"url":url,"title":bounded(v["title"].as_str().unwrap_or(""),300),"description":bounded(v["description"].as_str().unwrap_or(""),1000)}))}).collect();
            Ok(json!({"ok":true,"results":results,"untrustedSource":true}))
        })
    }
}
