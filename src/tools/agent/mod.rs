use super::*;
mod request_permissions;
mod request_user_input;
mod tool_search;
mod update_plan;
pub(crate) use update_plan::WorkingCheckpoint;
pub(super) fn register(r: &mut Registry) {
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
