use std::process::Command;

fn muxtop_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_muxtop"))
}

#[test]
fn test_about_flag() {
    let output = muxtop_cmd()
        .arg("--about")
        .output()
        .expect("failed to run muxtop --about");
    assert!(output.status.success(), "muxtop --about should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("muxtop v"), "Should print version");
    assert!(stdout.contains("MIT OR Apache-2.0"), "Should print license");
    assert!(stdout.contains("Privacy"), "Should print privacy pledge");
}

#[test]
fn test_version_flag() {
    let output = muxtop_cmd()
        .arg("--version")
        .output()
        .expect("failed to run muxtop --version");
    assert!(output.status.success(), "muxtop --version should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("muxtop"), "Should print muxtop name");
}

#[test]
fn test_help_flag() {
    let output = muxtop_cmd()
        .arg("--help")
        .output()
        .expect("failed to run muxtop --help");
    assert!(output.status.success(), "muxtop --help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--refresh"), "Should list --refresh flag");
    assert!(stdout.contains("--filter"), "Should list --filter flag");
    assert!(stdout.contains("--sort"), "Should list --sort flag");
    assert!(stdout.contains("--tree"), "Should list --tree flag");
    assert!(stdout.contains("--about"), "Should list --about flag");
}

#[test]
fn test_invalid_sort_field_fails() {
    let output = muxtop_cmd()
        .args(["--sort", "invalid", "--about"])
        .output()
        .expect("failed to run muxtop");
    assert!(
        !output.status.success(),
        "Invalid sort field should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid") || stderr.contains("error"),
        "Should print error for invalid sort field, got: {stderr}"
    );
}

#[test]
fn test_valid_sort_fields_accepted() {
    for field in ["cpu", "mem", "pid", "name", "user"] {
        let output = muxtop_cmd()
            .args(["--sort", field, "--about"])
            .output()
            .unwrap_or_else(|_| panic!("failed to run muxtop --sort {field} --about"));
        assert!(
            output.status.success(),
            "muxtop --sort {field} --about should exit 0, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
