use super::*;
mod apply_patch;
mod git_diff;
mod list_directory;
mod read_file;
mod search_files;
pub(super) fn register(r: &mut Registry) {
    r.register(
        "list_directory",
        Category::Files,
        Authority::WorkspaceRead,
        Arc::new(list_directory::ListDirectory),
    );
    r.register(
        "read_file",
        Category::Files,
        Authority::WorkspaceRead,
        Arc::new(read_file::ReadFile),
    );
    r.register(
        "search_files",
        Category::Files,
        Authority::WorkspaceRead,
        Arc::new(search_files::SearchFiles),
    );
    r.register(
        "apply_patch",
        Category::Files,
        Authority::Approval,
        Arc::new(apply_patch::ApplyPatch),
    );
    r.register(
        "git_diff",
        Category::Files,
        Authority::WorkspaceRead,
        Arc::new(git_diff::GitDiff),
    );
}
