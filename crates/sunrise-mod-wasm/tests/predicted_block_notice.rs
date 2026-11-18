//! The compiled wasm listeners answer like the native and wire-loopback ones.
#[allow(dead_code)]
#[path = "fixture/src/block_state_notice.rs"]
mod fixture;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[allow(dead_code)]
#[path = "fixture/src/block_path_type.rs"]
mod path_fixture;
#[path = "support/block_path_type.rs"]
mod path_support;
#[path = "support/block_state_notice.rs"]
mod support;
#[path = "../src/test_stack.rs"]
mod test_stack;
use sunrise_mod_package::manifest::Package;

/// The compiled fixture, with `probe` as an extra mod id its guest sees (which turns
/// on the registrations that probe asks for).
fn compiled_guest(probe: &str) -> (Vec<mod_api::PackageInfo>, sunrise_mod_wasm::Loaded) {
    let path = std::env::var("SUNRISE_WASM_FIXTURE").expect("build tests/fixture from this tree");
    let std = std::env::var("SUNRISE_WASM_STD").expect("build this tree's shared wasm std");
    let path = std::path::Path::new(&path);
    let mut manifest = Package::read(path).unwrap().manifest;
    manifest.inject.clear();
    let info = |id: &str| {
        mod_api::PackageInfo::new(
            id.into(),
            "0.1.0".into(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![id.into()],
            Vec::new(),
        )
    };
    let infos = vec![info(&manifest.id), info(probe)];
    let loaded = sunrise_mod_wasm::load(
        &std::fs::read(std).unwrap(),
        vec![sunrise_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.mods.len(), 1);
    loaded.initialize(&infos).unwrap();
    (infos, loaded)
}

#[test]
#[ignore = "daily"]
fn compiled_guest_reads_predicted_like_native() {
    test_stack::on_guest_stack(|| {
        let (infos, loaded) = compiled_guest("block_notice_probe");
        // The fixture guest declares a general entity type in `register`, which only the registration
        // phase accepts. The scope follows `initialize`: the phase lives in the backend it installs.
        mod_api::__private::initialize().unwrap();
        let _registration =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::install_packages(&infos, &[]);
            mod_api::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::mark_game_thread();
            let world = support::World::new();
            mod_api::__private::resolve_parts(
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
                mod_api::__private::KnownIds {
                    block: &|id| {
                        matches!(
                            id,
                            "minecraft:stone"
                                | "minecraft:air"
                                | "minecraft:water_cauldron"
                                | "wasm_fixture:part_state"
                        )
                    },
                    item: &|id| matches!(id, "minecraft:paper" | "minecraft:snowball"),
                },
            )
            .unwrap();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            world.check(&fixture::NoticePart);
            world.read.borrow_mut().clear();
            world.dispatch(true);
            world.dispatch(false);
            assert_eq!(*world.read.borrow(), [[1, 64, 1], [0, 64, 1]]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn compiled_guest_block_path_type_reads_saved_data_like_native() {
    test_stack::on_guest_stack(|| {
        let (infos, loaded) = compiled_guest("block_path_type_probe");
        mod_api::__private::initialize().unwrap();
        let _registration =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::install_packages(&infos, &[]);
            mod_api::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::mark_game_thread();
            let world = path_support::World::new();
            mod_api::__private::resolve_parts(
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
                mod_api::__private::KnownIds {
                    block: &|id| {
                        matches!(
                            id,
                            "minecraft:stone"
                                | "minecraft:air"
                                | "minecraft:water_cauldron"
                                | "wasm_fixture:part_state"
                        )
                    },
                    item: &|id| matches!(id, "minecraft:paper" | "minecraft:snowball"),
                },
            )
            .unwrap();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            // The same source natively, then through the installed compiled guest: both
            // read the neighbouring block entity's saved data and return Damaging.
            world.check(&path_fixture::PathPart);
            world.dispatch();
        });
    });
}
