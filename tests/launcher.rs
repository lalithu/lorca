use std::process::Command;

#[test]
fn rebuild_help_describes_complete_build_and_launch_without_starting_ui() {
    let result = Command::new(env!("CARGO_BIN_EXE_lorca"))
        .args(["rebuild", "--help"])
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(result.status.success());
    let text = String::from_utf8_lossy(&result.stdout);
    assert!(text.contains("Rust core, CLI, and macOS app"));
    assert!(text.contains("data are preserved"));
    assert!(text.contains("restart the app and backend"));
}

#[test]
fn rebuild_rejects_options_without_building_or_launching() {
    let result = Command::new(env!("CARGO_BIN_EXE_lorca"))
        .args(["rebuild", "--model", "test"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("takes no options"));
}

#[test]
fn normal_help_includes_rebuild_and_main_help_remains_available() {
    let result = Command::new(env!("CARGO_BIN_EXE_lorca"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("lorca rebuild"));
    let result = Command::new(env!("CARGO_BIN_EXE_lorca"))
        .args(["main", "--help"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("dashboard and Mini"));
}
