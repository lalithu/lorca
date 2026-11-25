use lorca::app::launcher;
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.first().and_then(|v| v.to_str()) {
        Some("serve") => lorca::transport::run(),
        Some("main") => launcher::run(args[1..].to_vec()),
        Some("rebuild") => launcher::rebuild(args[1..].to_vec()),
        Some("--help" | "-h") | None => {
            println!(
                "🌸 lorca main          Open the macOS app\nlorca rebuild [ui]   Rebuild the app\nlorca serve         Run the local chat backend"
            );
            Ok(())
        }
        _ => Err(std::io::Error::other(
            "This replacement currently supports main chat. Use lorca main.",
        )),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lorca: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
