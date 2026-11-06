use super::*;
mod compact_context;
mod history;
mod request_permissions;
mod request_user_input;
pub(crate) use request_user_input::RequestWorkspace;
mod tool_search;
mod update_plan;
pub(crate) use update_plan::WorkingCheckpoint;
pub(super) fn register(r: &mut Registry) {
    r.register(
        "compact_context",
        Category::Agent,
        Authority::TaskState,
        Arc::new(compact_context::CompactContext),
    );
    r.register(
        "search_history",
        Category::Agent,
        Authority::TaskState,
        Arc::new(history::HistoryTool::search(r.history.clone())),
    );
    r.register(
        "get_history_record",
        Category::Agent,
        Authority::TaskState,
        Arc::new(history::HistoryTool::read(r.history.clone())),
    );
    r.register(
        "tool_search",
        Category::Agent,
        Authority::TaskState,
        Arc::new(tool_search::ToolSearch::new(
            r.catalog.clone(),
            r.visible.clone(),
        )),
    );
    r.register(
        "update_plan",
        Category::Agent,
        Authority::TaskState,
        Arc::new(update_plan::UpdatePlan),
    );
    r.register(
        "request_user_input",
        Category::Agent,
        Authority::TaskState,
        Arc::new(request_user_input::RequestUserInput),
    );
    r.register(
        "request_permissions",
        Category::Agent,
        Authority::TaskState,
        Arc::new(request_permissions::RequestPermissions),
    );
}
