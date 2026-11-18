//! The native part and the real generated guest dispatcher both receive the read-only
//! level host and read the same neighbouring block entity.
use super::*;
#[path = "../../../tests/fixture/src/block_path_type.rs"]
mod fixture;
#[path = "../../../tests/support/block_path_type.rs"]
mod support;

struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.block_key("minecraft:stone");
    }
}

#[test]
fn wire_loopback_block_path_type_reads_the_level_like_native() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("path-type", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let world = support::World::new();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        world.check(&fixture::PathPart);
        let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockPart>(Arc::new(
            fixture::PathPart,
        ))]);
        let proxy = WasmHandler {
            index,
            module: WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            },
        };
        world.check(&proxy);
    });
}
