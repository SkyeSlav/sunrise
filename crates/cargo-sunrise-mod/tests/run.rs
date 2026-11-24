use std::{path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn run_of_a_native_mod_without_resources_says_how_to_make_them_before_building() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("native-run-resources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // A native mod's folder, made by hand: the resources are asked for before anything is fetched or built.
    let project = dir.join("machine");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = 'machine'\nversion = '0.1.0'\n",
    )
    .unwrap();
    let manifest = include_str!("../template/native/sunrise-mod.toml")
        .replace("{id}", "machine")
        .replace("{name}", "Machine");
    std::fs::write(
        project.join("sunrise-mod.toml"),
        format!(
            "{manifest}\n[native]\ngame = \"26.3\"\nsunrise = \"{0}\"\nmixins = \"{0}\"\n",
            "a".repeat(40)
        ),
    )
    .unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(&project)
        .arg("run")
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--resources <folder>"), "{error}");
    assert!(error.contains("sunrise-prepare --prepare"), "{error}");
    // A native mod runs on its pinned SDK's game: `--game` is refused, not dropped. Without
    // resources, a run that did not refuse it would stop at them, before any SDK is looked up.
    let refused = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(&project)
        .args(["run", "--game", "some-game"])
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--game"), "{error}");
    assert!(
        !project.join("target").exists(),
        "run must stop before building"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn content_run_keeps_the_previous_package_on_jar_refusal_and_starts_with_the_staged_home() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use sunrise_mod_package::Package;

    const SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                          2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
    let scratch = tempfile::tempdir().unwrap();
    let project = scratch.path().join("garden");
    fs::create_dir(&project).unwrap();
    let manifest = project.join("sunrise-mod.toml");
    let text = "id = 'garden'\nname = 'Garden'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n";
    fs::write(&manifest, text).unwrap();
    let previous = Package::pack_directory(&project).unwrap();
    let output = project.join("target/sunrise-mod/garden.smod");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, &previous).unwrap();
    fs::write(
        &manifest,
        format!("{text}\n[[jar]]\nid = 'original'\nversion = '1.0'\nsha512 = '{SHA512}'\n"),
    )
    .unwrap();

    let source = scratch.path().join("originals");
    fs::create_dir(&source).unwrap();
    let game = scratch.path().join("fake-game");
    fs::write(
        &game,
        format!("#!/bin/sh\nset -eu\ntouch launched\ntest \"$1\" = --dev-mod\ntest -f \"$2\"\ntest \"$(cat \"$SUNRISE_HOME/jars/{SHA512}.jar\")\" = abc\n"),
    )
    .unwrap();
    fs::set_permissions(&game, fs::Permissions::from_mode(0o700)).unwrap();
    let run_with = |more: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(&project)
            .env("SUNRISE_HOME", scratch.path().join("player-home"))
            .arg("run")
            .arg("--game")
            .arg(&game)
            .arg("--jars")
            .arg(&source)
            .args(more)
            .output()
            .unwrap()
    };
    let run = || run_with(&[]);

    let refused = run();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(
        error.contains("original") && error.contains(SHA512),
        "{error}"
    );
    assert!(!project.join("launched").exists());
    assert_eq!(fs::read(&output).unwrap(), previous);

    fs::write(source.join(format!("{SHA512}.jar")), b"abc").unwrap();

    // The tool reads `--resources` for a native mod only: for this one it is refused, not dropped.
    let refused = run_with(&["--resources", "."]);
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--resources"), "{error}");
    assert!(!project.join("launched").exists());

    let ran = run();
    assert!(ran.status.success(), "{ran:?}");
    assert!(project.join("launched").is_file());
    assert_eq!(
        Package::read(&output).unwrap().manifest.jar[0].sha512(),
        SHA512
    );
}
