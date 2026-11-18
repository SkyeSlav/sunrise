//! A newcomer's first command: no command, `--help` and `-h` print the usage and succeed; an unknown command fails.
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
#[ignore = "daily"]
fn help_prints_the_usage_to_standard_output_and_succeeds() {
    for args in [&[][..], &["--help"], &["-h"], &["sunrise-mod", "--help"]] {
        let output = run(args);
        assert!(output.status.success(), "{args:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.starts_with("usage: cargo sunrise-mod new <path>"),
            "{args:?}: {text}"
        );
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
#[ignore = "daily"]
fn an_unknown_command_is_an_error() {
    let output = run(&["frobnicate"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: cargo sunrise-mod new"));
}
