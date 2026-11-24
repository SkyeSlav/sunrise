//! The command registries the game keeps in step with what the world's content and data
//! packs define, outside the one-time build in `chat.rs`.
use super::{Game, chat::TimeMarker};
use std::collections::BTreeMap;

/// Extracts every timeline's `(clock, period)` and every clock's time markers from the world's
/// `minecraft:timeline` table (`ServerClockManager.init`).
pub(super) fn timelines_and_markers(
    table: &sunrise_client::environment::TimelineTable,
) -> (
    BTreeMap<String, (String, Option<i64>)>,
    BTreeMap<(String, String), TimeMarker>,
) {
    let mut timelines = BTreeMap::new();
    let mut markers = BTreeMap::new();
    for (id, timeline) in &table.timelines {
        let period = timeline.period.map(i64::from);
        timelines.insert(id.clone(), (timeline.clock.clone(), period));
        for (marker_id, info) in &timeline.time_markers {
            markers.insert(
                (timeline.clock.clone(), marker_id.clone()),
                TimeMarker {
                    ticks: i64::from(info.ticks),
                    period,
                    show: info.show_in_commands,
                },
            );
        }
    }
    (timelines, markers)
}

impl Game<'_> {
    /// Active `minecraft:timeline` and `minecraft:world_clock` snapshot for `/time` commands
    /// and `wake_time` (`ServerClockManager.init`, `TimeCommand`).
    pub(super) fn sync_timeline_registries(&mut self) {
        let (timelines, markers) = self
            .world_tables
            .as_ref()
            .map_or_else(Default::default, |tables| {
                timelines_and_markers(&tables.timelines)
            });
        self.ui.timelines = timelines;
        self.ui.markers = markers;
        let reg = &mut self.ui.registries;
        let timeline_ids: Vec<String> = self.ui.timelines.keys().cloned().collect();
        reg.timelines.clone_from(&timeline_ids);
        reg.by_registry
            .entry("minecraft:timeline".into())
            .or_default()
            .ids = timeline_ids;
        let mut command_markers: Vec<String> = self
            .ui
            .markers
            .iter()
            .filter(|(_, m)| m.show)
            .map(|((_, marker), _)| marker.clone())
            .collect();
        command_markers.sort();
        command_markers.dedup();
        reg.time_markers = command_markers;
        if let Some(tables) = &self.world_tables {
            for (registry, id, admitted) in tables.packed_registries.entries() {
                if registry == "minecraft:world_clock" && admitted && !reg.clocks.contains(&id) {
                    reg.clocks.push(id);
                }
            }
            reg.clocks.sort();
            reg.clocks.dedup();
            reg.by_registry
                .entry("minecraft:world_clock".into())
                .or_default()
                .ids
                .clone_from(&reg.clocks);
        }
    }

    /// Active enchantment registry snapshot for command codecs and completions.
    /// Replace both ID views; keep the existing registry tags intact.
    pub(super) fn sync_enchantment_registries(&mut self) {
        let mut ids: Vec<String> = self
            .active_enchantment_definitions()
            .list
            .iter()
            .map(|entry| entry.id.to_string())
            .collect();
        ids.sort();
        ids.dedup();
        self.ui.registries.enchantments = ids.clone();
        self.ui
            .registries
            .by_registry
            .entry("minecraft:enchantment".into())
            .or_default()
            .ids = ids;
    }
}
