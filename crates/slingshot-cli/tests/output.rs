use std::process::{Command, Output};

fn invoke(args: &[&str], no_color: bool, term: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_slingshot"));
    command.args(args).env_remove("NO_COLOR").env("TERM", term);
    if no_color {
        command.env("NO_COLOR", "1");
    }
    command.output().unwrap()
}

#[test]
fn errors_use_stderr_and_respect_color_modes() {
    for (args, no_color, term, colored) in [
        (vec!["run"], false, "xterm", false),
        (vec!["--color", "always", "run"], true, "dumb", true),
        (vec!["--color", "never", "run"], false, "xterm", false),
    ] {
        let output = invoke(&args, no_color, term);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("No command given"));
        assert_eq!(error.contains('\x1b'), colored);
    }
}

#[test]
fn help_is_readable_and_uses_stdout() {
    for command in [
        None,
        Some("run"),
        Some("health"),
        Some("link"),
        Some("attach"),
        Some("sync"),
        Some("env"),
        Some("ps"),
        Some("stop"),
        Some("menubar"),
    ] {
        let mut args = vec!["--color", "never"];
        if let Some(command) = command {
            args.push(command);
        }
        args.push("--help");
        let output = invoke(&args, false, "xterm");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Usage:"));
        assert!(!help.contains('\x1b'));
        assert!(!help.contains("trailing_var_arg"));
    }
}

#[test]
fn explicit_color_applies_to_help() {
    let output = invoke(&["--color=always", "--help"], true, "dumb");
    assert!(output.status.success());
    assert!(output.stdout.contains(&0x1b));
}

#[test]
fn internal_helpers_are_hidden_from_help() {
    let output = invoke(&["--color", "never", "--help"], false, "xterm");
    let help = String::from_utf8(output.stdout).unwrap();
    for visible in ["attach", "sync", "env", "ps", "stop", "top"] {
        assert!(help.contains(visible), "{visible} missing from help");
    }
    assert!(!help.contains("internal"));
}

#[test]
fn interactive_commands_refuse_without_a_terminal() {
    let output = invoke(&["--color", "never", "attach"], false, "xterm");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("needs an interactive terminal"), "{error}");
}
