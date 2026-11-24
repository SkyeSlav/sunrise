use super::*;

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn the_cache_is_the_override_else_the_platforms_cache_folder() {
    let pairs = [("SUNRISE_MOD_CACHE", "/mine"), ("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/mine")
    );
    let pairs = [("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/xdg/sunrise-mod")
    );
    let pairs = [("HOME", "/home/a")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Unix).unwrap(),
        PathBuf::from("/home/a/.cache/sunrise-mod")
    );
    assert_eq!(
        cache_in(&env(&pairs), Platform::Mac).unwrap(),
        PathBuf::from("/home/a/Library/Caches/sunrise-mod")
    );
    let pairs = [("LOCALAPPDATA", "C:/Local")];
    assert_eq!(
        cache_in(&env(&pairs), Platform::Windows).unwrap(),
        PathBuf::from("C:/Local/sunrise-mod")
    );
    let error = cache_in(&env(&[]), Platform::Unix).unwrap_err();
    assert!(error.contains("SUNRISE_MOD_CACHE"), "{error}");
}

#[test]
fn the_rust_is_rustups_else_the_exact_one_on_the_path_else_an_error_saying_how() {
    let target = "x86_64-unknown-linux-gnu";
    let found = |version: &str, host: &str| Found {
        version: version.into(),
        host: host.into(),
        sysroot: "/sys".into(),
    };
    assert_eq!(
        choose("1.99.1", target, true, Some(&found("1.80.0", target))).unwrap(),
        Source::Rustup("1.99.1-x86_64-unknown-linux-gnu".into())
    );
    assert_eq!(
        choose("1.99.1", target, false, Some(&found("1.99.1", target))).unwrap(),
        Source::System
    );
    for other in [
        Some(found("1.98.0", target)),
        Some(found("1.99.1", "aarch64-apple-darwin")),
        None,
    ] {
        let error = choose("1.99.1", target, false, other.as_ref()).unwrap_err();
        assert!(
            error.contains("1.99.1") && error.contains("rustup.rs"),
            "{error}"
        );
    }
}

#[test]
fn the_staged_record_is_the_builds_libraries_as_the_loader_reads_them() {
    let row = Library {
        file: "libmod_greeter.so".into(),
        sha256: "ab".repeat(32),
        kind: "mod".into(),
        mod_id: "greeter".into(),
        identity: "b greeter 0.1.0 1.99.1".into(),
    };
    let json: serde_json::Value = serde_json::from_str(&record(&[row]).unwrap()).unwrap();
    assert_eq!(json["libraries"][0]["file"], "libmod_greeter.so");
    assert_eq!(json["libraries"][0]["mod"], "greeter");
    assert_eq!(json["libraries"][0]["identity"], "b greeter 0.1.0 1.99.1");
}

/// The Motion Blur 0.1.5 case: the pin moved, the SDK's `sunrise-core` changed its dependencies,
/// and the committed lock still named the old ones.
#[test]
#[ignore = "daily"]
fn a_lock_the_pinned_sdk_moved_is_refused_until_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let package = |path: &str, name: &str, dependencies: &str| {
        write(
            &format!("{path}Cargo.toml"),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{dependencies}"
            ),
        );
        write(&format!("{path}src/lib.rs"), "");
    };
    let sdk = ".sunrise/sdk/api/crates/";
    package(
        "",
        "mod-fixture",
        "sunrise-mod-api = { path = \".sunrise/sdk/api/crates/sunrise-mod-api\" }\n",
    );
    package(
        &format!("{sdk}sunrise-mod-api/"),
        "sunrise-mod-api",
        "sunrise-core = { path = \"../sunrise-core\" }\n",
    );
    // Local stand-ins for the registry packages, so Cargo needs no network.
    package(&format!("{sdk}crc-fast/"), "crc-fast", "");
    package(&format!("{sdk}stacker/"), "stacker", "");
    let core = |dependency: &str| {
        package(
            &format!("{sdk}sunrise-core/"),
            "sunrise-core",
            &format!("{dependency} = {{ path = \"../{dependency}\" }}\n"),
        )
    };
    core("crc-fast");
    let found = found().expect("a rustc on PATH");
    let rust = Unpacked {
        dir: found.sysroot,
        target: found.host,
        cargo_home: root.join("cargo-home"),
        cargo_config: Vec::new(),
        c_bin: None,
    };
    refresh_lock(&rust, root).unwrap();
    check_lock(&rust, root, "old").unwrap();

    // The new pin's SDK.
    core("stacker");
    let stale = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let error = check_lock(&rust, root, "new").unwrap_err();
    assert!(error.contains("pinned SDK (Sunrise new)"), "{error}");
    assert!(error.contains("cargo sunrise-mod pin new"), "{error}");
    assert_eq!(fs::read_to_string(root.join("Cargo.lock")).unwrap(), stale);

    refresh_lock(&rust, root).unwrap();
    check_lock(&rust, root, "new").unwrap();
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    assert!(
        lock.contains("\"stacker\"") && !lock.contains("crc-fast"),
        "{lock}"
    );
}
