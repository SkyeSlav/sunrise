//! A wasm mod imports the symbols of the members before it: the std member exports std's. Whatever
//! the host that built either side, every import of the greeter template must be an export of the
//! std member (the symbol disambiguator of std units carries nothing of the host).
use std::{collections::BTreeSet, path::Path, process::Command};
use wasmparser::{Parser, Payload, TypeRef};

#[test]
#[ignore = "daily"]
fn every_import_of_a_built_greeter_is_an_export_of_the_std_member() {
    // Outside the checkout: a project inside it would believe it is in the game's workspace.
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    let tool = |current: &Path, args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(current)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let std_member = dir.join("std.wasm");
    tool(dir, &["wasm-std", std_member.to_str().unwrap()]);
    tool(dir, &["new", "greeter", "--template", "wasm"]);
    let greeter = dir.join("greeter");
    tool(&greeter, &["build", "--wasm"]);

    let mut exports = BTreeSet::new();
    for payload in Parser::new(0).parse_all(&std::fs::read(&std_member).unwrap()) {
        if let Payload::ExportSection(section) = payload.unwrap() {
            exports.extend(section.into_iter().map(|e| e.unwrap().name.to_owned()));
        }
    }
    let member = greeter.join("mod.wasm");
    let mut missing = Vec::new();
    let mut checked = 0;
    for payload in Parser::new(0).parse_all(&std::fs::read(member).unwrap()) {
        if let Payload::ImportSection(section) = payload.unwrap() {
            for import in section.into_imports().map(Result::unwrap) {
                // Memory, table and stack pointer come from the group's linker, not from a member.
                if import.module == "env" && matches!(import.ty, TypeRef::Func(_)) {
                    checked += 1;
                    if !exports.contains(import.name) {
                        missing.push(import.name.to_owned());
                    }
                }
            }
        }
    }
    assert!(checked > 0, "the greeter imports nothing from std");
    assert!(missing.is_empty(), "not exported by std: {missing:?}");
}
