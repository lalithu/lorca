mod chat;
mod config;
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
    let Some(config) = config::Config::load()? else {
        return Ok(());
    };
    let mut model = model::Ollama::new(&config)?;
    ui::run(&config, &mut model)
}
