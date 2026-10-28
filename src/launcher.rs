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
pub fn rebuild(args: Vec<OsString>) -> io::Result<()> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Usage: lorca rebuild\n\nQuit lorca first. Rebuild the Rust core, CLI, and macOS app, update the installed CLI, then open the dashboard.\nUses the repository this CLI was installed from; works from any directory."
        );
        return Ok(());
    }
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lorca rebuild takes no options. Use lorca rebuild --help.",
        ));
    }
    rebuild_app()
}

#[cfg(target_os = "macos")]
fn rebuild_app() -> io::Result<()> {
    use std::{path::PathBuf, process::Command};

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = root.join("macos/build.sh");
    if !script.is_file() || !root.join("Cargo.toml").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Source repository not found. Reinstall lorca with cargo install --path . --locked --force from your current checkout.",
        ));
    }
    println!("Rebuilding lorca in {}…", root.display());
    let status = Command::new("/bin/bash")
        .arg(script)
        .env("LORCA_INSTALL_CLI", "1")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err(io::Error::other(
            "build failed; the dashboard was not opened",
        ));
    }
    println!("Rebuild complete. Opening lorca…");
    open_app()
}

#[cfg(not(target_os = "macos"))]
fn rebuild_app() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "lorca rebuild builds the native macOS app and requires macOS",
    ))
}
