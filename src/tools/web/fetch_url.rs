use super::http;
use crate::tools::{arguments::parse, *};
pub struct FetchUrl;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    url: String,
}
impl Tool for FetchUrl {
    fn schema(&self) -> Value {
        schema(
            "fetch_url",
            "Fetch a public HTTPS page as bounded source text/HTML. Redirects and DNS are checked; private networks, cookies and authentication are unavailable. Requires approval.",
            json!({"url":{"type":"string"}}),
            vec!["url"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let args: Args = parse(a.clone())?;
        http::url(&args.url)?;
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
            let (url, ty, text, truncated) = http::request(&args.url, None, None, c).await?;
            Ok(
                json!({"ok":true,"url":url,"contentType":ty,"content":bounded(&text,8000),"truncated":truncated||text.len()>8000,"untrustedSource":true}),
            )
        })
    }
}
