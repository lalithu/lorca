use super::*;
mod fetch_url;
mod http;
mod web_search;
pub(super) fn register(r: &mut Registry) {
    r.register(
        "fetch_url",
        Category::Web,
        Authority::PublicWeb,
        Arc::new(fetch_url::FetchUrl),
    );
    match std::env::var("LORCA_WEB_SEARCH_URL") {
  Ok(url) if http::url(&url).is_ok()=>r.register("web_search",Category::Web,Authority::PublicWeb,Arc::new(web_search::WebSearch{endpoint:url,token:std::env::var("LORCA_WEB_SEARCH_TOKEN").ok()})),
  _=>r.mark_unavailable("web_search","No valid public HTTPS JSON search endpoint configured (LORCA_WEB_SEARCH_URL). Job Apps keeps its independent existing Exa integration."),
 }
}
