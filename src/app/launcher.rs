use std::{ffi::OsString, io};

pub fn run(args: Vec<OsString>) -> io::Result<()> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("Usage: lorca main\n\nOpen the macOS dashboard and Mini.");
        return Ok(());
    }
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lorca main takes no options. Use lorca main --help.",
        ));
    }
    open_app()
}

#[cfg(target_os = "macos")]
fn open_app() -> io::Result<()> {
    use std::{env, path::PathBuf, process::Command};

    let mut candidates =
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("macos/build/lorca.app")];
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("Applications/lorca.app"));
    }
    candidates.push(PathBuf::from("/Applications/lorca.app"));
    let app = candidates.into_iter().find(|path| path.is_dir()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "macOS app not found. Run macos/build.sh in the repository, or install lorca.app in ~/Applications or /Applications.",
        )
    })?;
    let status = Command::new("/usr/bin/open").arg(app).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("could not open the macOS app"))
    }
}

#[cfg(not(target_os = "macos"))]
fn open_app() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the dashboard and Mini require macOS; run lorca for terminal chat",
    ))
}

/// Explicit developer command; ordinary launches remain quiet and never build.
pub fn rebuild(mut args: Vec<OsString>) -> io::Result<()> {
    let ui_only = args.first().is_some_and(|arg| arg == "ui");
    if ui_only {
        args.remove(0);
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Usage: lorca rebuild [ui]\n\nRebuild the Rust core, CLI, and macOS app, update the installed CLI, then restart the app and backend. Conversations, drafts, settings and Job Apps data are preserved.\nUse `lorca rebuild ui` to incrementally build only Swift in debug mode and relaunch the UI, reusing the bundled Rust core and running backend. Run a full rebuild first.\nUses the repository this CLI was installed from; works from any directory."
        );
        return Ok(());
    }
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lorca rebuild takes no options except the ui subcommand. Use lorca rebuild --help.",
        ));
    }
    rebuild_app(ui_only)
}

#[cfg(target_os = "macos")]
fn rebuild_app(ui_only: bool) -> io::Result<()> {
    use std::{path::PathBuf, process::Command};

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = root.join("macos/build.sh");
    if !script.is_file() || !root.join("Cargo.toml").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Source repository not found. Reinstall lorca with cargo install --path . --locked --force from your current checkout.",
        ));
    }
    println!(
        "Rebuilding lorca{} in {}…",
        if ui_only { " UI" } else { "" },
        root.display()
    );
    let status = Command::new("/bin/bash")
        .arg(script)
        .env("LORCA_INSTALL_CLI", if ui_only { "0" } else { "1" })
        .env("LORCA_UI_ONLY", if ui_only { "1" } else { "0" })
        .env("LORCA_BUILD_ONLY", "0")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(
            "build or restart failed; a fresh restart was not confirmed",
        ));
    }
    if ui_only {
        println!("UI rebuild complete. App relaunched using the existing Rust core.");
    } else {
        println!("Rebuild complete. Fresh app and backend restart verified.");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn rebuild_app(_ui_only: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "lorca rebuild builds the native macOS app and requires macOS",
    ))
}
