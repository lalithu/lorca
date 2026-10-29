use crate::tools::registry::{Catalog, Visible};
use crate::tools::{arguments::parse, *};
pub struct ToolSearch {
    catalog: Catalog,
    visible: Visible,
}
impl ToolSearch {
    pub(super) fn new(catalog: Catalog, visible: Visible) -> Self {
        Self { catalog, visible }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    query: String,
}

impl Tool for ToolSearch {
    fn schema(&self) -> Value {
        schema(
            "tool_search",
            "Find registered tools by name, description or drawer; expose up to five matching schemas for subsequent turns. Discovery never grants authority. Search an empty query to list the catalog.",
            json!({"query":{"type":"string","maxLength":200}}),
            vec!["query"],
        )
    }
    fn prepare(&self, _: &Workspace, a: Value, l: &Limits) -> Result<Prepared> {
        self.prepare_optional(None, a, l)
    }
    fn prepare_optional(&self, _: Option<&Workspace>, a: Value, _: &Limits) -> Result<Prepared> {
        check_size(&a)?;
        let a: Args = parse(a.clone())?;
        if a.query.len() > 200 {
            return Err("Query exceeds 200 bytes".into());
        }
        prepared(a_as_value(a.query), false)
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
            if *c.borrow() {
                return Err("Cancelled".into());
            }
            let query = p.payload["query"].as_str().unwrap_or("").to_lowercase();
            let catalog = self.catalog.read().unwrap();
            let terms: Vec<_> = query.split_whitespace().collect();
            let mut found: Vec<_> = catalog
                .values()
                .filter_map(|m| {
                    let hay =
                        format!("{} {} {:?}", m.name, m.description, m.category).to_lowercase();
                    let score = terms.iter().filter(|term| hay.contains(**term)).count()
                        + usize::from(m.name == query) * 10;
                    if terms.is_empty() || score > 0 {
                        Some((score, m))
                    } else {
                        None
                    }
                })
                .collect();
            found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
            let total = found.len();
            let matches: Vec<_> = found
                .iter()
                .map(|(_, m)| json!({"name":m.name,"category":m.category}))
                .collect();
            let mut visible = self.visible.lock().unwrap();
            let results:Vec<_>=found.into_iter().take(5).map(|(_,m)|{visible.insert(m.name.clone());json!({"name":m.name,"description":m.description,"category":m.category,"authority":m.authority,"schema":m.schema})}).collect();
            Ok(
                json!({"ok":true,"tools":results,"more":total.saturating_sub(5),"matches":matches,"note":"Schemas are visible next turn; existing workspace restrictions and approvals still apply."}),
            )
        })
    }
}

fn a_as_value(query: String) -> Value {
    json!({"query":query})
}
