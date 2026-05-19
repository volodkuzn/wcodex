use std::process::Command;

#[test]
fn doctor_runs_when_integration_is_enabled() {
    if std::env::var("WCODEX_INTEGRATION").as_deref() != Ok("1") {
        return;
    }

    let status = Command::new(env!("CARGO_BIN_EXE_wcodex"))
        .arg("doctor")
        .status()
        .expect("failed to run wcodex doctor");
    assert!(status.success());
}
