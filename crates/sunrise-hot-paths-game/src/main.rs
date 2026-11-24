//! The game as `tools/hot_paths.py` measures it (docs/hot-path-trends.md). Without
//! `SUNRISE_HOT_PATHS_MODS=native` it starts exactly as `sunrise-game` does, with no mod. With it,
//! it is what the launcher's generated `main` builds for one native mod: `hot_paths_listener`
//! listens to the entity tick hooks and does nothing in them, so the benchmark's hook workload
//! measures a dispatch that reaches a listener. That run needs a
//! `sunrise-native-mods/hot_paths_listener` package beside the executable, which the tool writes.
//!
//! A crate and not a feature of `sunrise-game`: the game players get links no mod.
#![forbid(unsafe_code)]
use mod_api::__private::native::BuiltMod;
use mod_api::Side;
use mod_api::{
    EntityTickListenedEvent, EntityTickPostEvent, EntityTickPreEvent, Hooks, HostSide, ModEntry,
};

const ID: &str = "hot_paths_listener";
const BUILT: &[BuiltMod] = &[BuiltMod {
    id: ID,
    version: "1.0.0",
    side: Side::Both,
}];

struct Listener;

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_tick_listened(&self, _: HostSide<'_>, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }
    fn entity_tick_pre(&self, _: HostSide<'_>, _: &mut EntityTickPreEvent<'_>) {}
    fn entity_tick_post(&self, _: HostSide<'_>, _: &mut EntityTickPostEvent<'_>) {}
}

fn main() {
    sunrise_mod_host::prepare_main_stack();
    let startup = sunrise_client::entry::prepare();
    std::hint::black_box(&sunrise_mod_package::holder::CAPABILITY);
    if std::env::var_os("SUNRISE_HOT_PATHS_MODS").is_some_and(|mods| mods == "native") {
        sunrise_mod_host::main_with(vec![ModEntry::new(ID, Box::new(Listener))], BUILT, startup);
    } else {
        sunrise_mod_host::main_with(Vec::new(), &[], startup);
    }
}
