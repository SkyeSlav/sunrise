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
    assert!(
        !project.join("target").exists(),
        "run must stop before building"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
