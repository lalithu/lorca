use super::*;
pub(super) mod command;
mod exec_command;
mod sessions;
mod write_stdin;
pub(super) fn register(r: &mut Registry) {
    let sessions = Arc::new(sessions::Sessions::default());
    r.register(
        "exec_command",
        Category::Terminal,
        Authority::Approval,
        Arc::new(exec_command::ExecCommand {
            sessions: sessions.clone(),
            legacy: false,
        }),
    );
    r.register(
        "write_stdin",
        Category::Terminal,
        Authority::Approval,
        Arc::new(write_stdin::WriteStdin {
            sessions: sessions.clone(),
        }),
    );
    // Persisted conversations and third-party callers retain their old command name/schema.
    r.register(
        "run_command",
        Category::Terminal,
        Authority::Approval,
        Arc::new(exec_command::ExecCommand {
            sessions,
            legacy: true,
        }),
    );
}
