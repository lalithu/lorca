mod agent_cli;
mod config;
mod job_session;
mod launcher;
mod lifecycle;
mod model;
mod timer;
mod ui;

use std::{io, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lorca: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    if std::env::args().nth(1).as_deref() == Some("job-session") {
        return job_session::run();
    }
    if std::env::args().nth(1).as_deref() == Some("agent") {
        return agent_cli::run();
    }
    if std::env::args().nth(1).as_deref() == Some("serve") {
        return lorca::transport::run();
    }
    match args.next().as_deref() {
        Some(command) if command == "main" => return launcher::run(args.collect()),
        Some(command) if command == "rebuild" => return launcher::rebuild(args.collect()),
        _ => {}
    }
    let Some(config) = config::Config::load()? else {
        return Ok(());
    };
    let mut model = model::Ollama::new(&config)?;
    ui::run(&config, &mut model)
}
