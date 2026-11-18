//! #792 stage 4b round 1: the five native-only `EntityPart` methods refuse a
//! wasm mod that overrides one, by name and with the reason, through the
//! registration refusal channel — never a silent accept of an override
//! nothing would call. The guest's attach wrapper reads the subscription
//! macro's `entity_native_only` metadata where the real part lives, so these
//! run the real generated guest and host boundaries, and a tick-only part
//! attaches clean beside them.
use super::*;
use mod_api::{EntityPart, Registrar};
use std::cell::Cell;

/// Which entity part this file's registration attaches: none by default, so
/// its refusing parts never disturb another test's registration.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Attach {
    None,
    Goals,
    Behaviors,
    ClusterSize,
    Shearable,
    Sheared,
    TickOnly,
}

thread_local! {
    pub(super) static ATTACH: Cell<Attach> = const { Cell::new(Attach::None) };
}

/// The guest's parts: one overriding each native-only method, and one
/// tick-only that must attach clean.
struct Goals;
#[mod_api::sunrise_part]
impl EntityPart for Goals {
    fn register_goals(&self, _: &mut mod_api::GoalRegistrar<'_>) {}
}
struct Behaviors;
#[mod_api::sunrise_part]
impl EntityPart for Behaviors {
    fn register_behaviors(&self, _: &mut mod_api::BehaviorRegistrar<'_>) {}
}
struct ClusterSize;
#[mod_api::sunrise_part]
impl EntityPart for ClusterSize {
    fn spawn_cluster_size(
        &self,
        _: &dyn mod_api::Host,
        _: mod_api::UnspawnedMobRef,
        _: mod_api::SpawnReason,
        _: Option<mod_api::SpawnSource>,
        _: u32,
    ) -> Option<u32> {
        None
    }
}
struct Shearable;
#[mod_api::sunrise_part]
impl EntityPart for Shearable {
    fn is_shearable(
        &self,
        _: &dyn mod_api::Host,
        _: mod_api::EntityRef,
        _: Option<mod_api::PlayerRef>,
        _: &mod_api::ItemStack,
    ) -> Option<bool> {
        None
    }
}
struct Sheared;
#[mod_api::sunrise_part]
impl EntityPart for Sheared {
    fn on_sheared(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: mod_api::EntityRef,
        _: Option<mod_api::PlayerRef>,
        _: &mod_api::ItemStack,
    ) -> Vec<mod_api::ItemStack> {
        Vec::new()
    }
}
struct TickOnly;
#[mod_api::sunrise_part]
impl EntityPart for TickOnly {
    fn general_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::GeneralEntityTickEvent<'_>,
    ) {
    }
}

pub(super) struct EntityPartNativeOnlyHooks;
impl EntityPartNativeOnlyHooks {
    pub(super) fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        match ATTACH.get() {
            Attach::None => {}
            Attach::Goals => registrar.entity_part("wire_test:robot", Goals),
            Attach::Behaviors => registrar.entity_part("wire_test:robot", Behaviors),
            Attach::ClusterSize => registrar.entity_part("wire_test:robot", ClusterSize),
            Attach::Shearable => registrar.entity_part("wire_test:robot", Shearable),
            Attach::Sheared => registrar.entity_part("wire_test:robot", Sheared),
            Attach::TickOnly => registrar.entity_part("wire_test:robot", TickOnly),
        }
    }
}

/// Install the loopback mod with this file's registration attaching `attach`:
/// the guest's register runs through the real generated boundary, so the
/// refusal its attach wrapper delivers lands in the registrar the game would
/// refuse, and the gate closes again for the next test.
fn install_with(attach: Attach) -> Result<(), Vec<String>> {
    ATTACH.set(attach);
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        }),
    )];
    let result = mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    );
    ATTACH.set(Attach::None);
    result
}

/// The refusal names the method and the reason, and refuses the registration.
fn refuses_by_name(attach: Attach, method: &str) {
    let error = install_with(attach).unwrap_err();
    let text = error.join("\n");
    assert!(text.contains("refused registration"), "{text}");
    assert!(text.contains(&format!("EntityPart::{method}:")), "{text}");
    assert!(
        text.contains("cannot override native-only methods"),
        "{text}"
    );
}

#[test]
fn wire_loopback_an_entity_part_overriding_register_goals_refuses_its_load_by_name() {
    refuses_by_name(Attach::Goals, "register_goals");
}

#[test]
fn wire_loopback_an_entity_part_overriding_register_behaviors_refuses_its_load_by_name() {
    refuses_by_name(Attach::Behaviors, "register_behaviors");
}

#[test]
fn wire_loopback_an_entity_part_overriding_spawn_cluster_size_refuses_its_load_by_name() {
    refuses_by_name(Attach::ClusterSize, "spawn_cluster_size");
}

#[test]
fn wire_loopback_an_entity_part_overriding_is_shearable_refuses_its_load_by_name() {
    refuses_by_name(Attach::Shearable, "is_shearable");
}

#[test]
fn wire_loopback_an_entity_part_overriding_on_sheared_refuses_its_load_by_name() {
    refuses_by_name(Attach::Sheared, "on_sheared");
}

#[test]
fn wire_loopback_a_tick_only_entity_part_loads_without_a_refusal() {
    install_with(Attach::TickOnly).unwrap();
}
