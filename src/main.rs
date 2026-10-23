mod chat;
mod config;
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
    if args.next().as_deref() == Some(std::ffi::OsStr::new("main")) {
        return launcher::run(args.collect());
    }
    let Some(config) = config::Config::load()? else {
        return Ok(());
    };
    let mut model = model::Ollama::new(&config)?;
    ui::run(&config, &mut model)
}
