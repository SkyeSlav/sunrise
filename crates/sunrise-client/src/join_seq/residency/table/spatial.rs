//! Spatial candidates in `EntitySectionStorage` order, independent of lookup admission order.
use super::{DimensionKey, JoinSeq, ResidentRow};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    ops::Bound::{Excluded, Included, Unbounded},
};
use sunrise_core::{fast_hash::FxHashMap, geometry::Aabb, mth, position::SectionPos};

/// Absolute X and the packed section key. Retaining X avoids wrapping a finite
/// query (or an entity outside the usual world range) at `SectionPos`'s bit boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SectionKey(i32, i64);

impl SectionKey {
    pub fn at([x, y, z]: [f64; 3]) -> Self {
        let pos = section_at([x, y, z]);
        Self(pos.x, pos.pack())
    }
}

fn section_at([x, y, z]: [f64; 3]) -> SectionPos {
    SectionPos::new(mth::floor(x) >> 4, mth::floor(y) >> 4, mth::floor(z) >> 4)
}

/// `EntitySectionStorage.forEachAccessibleNonEmptySection` search bounds.
#[derive(Clone, Copy)]
pub struct SectionSearch {
    min: SectionPos,
    max: SectionPos,
}

impl SectionSearch {
    pub fn new(bounds: Aabb) -> Self {
        let min = bounds.min_position();
        let max = bounds.max_position();
        Self {
            min: section_at([min.x - 2.0, min.y - 4.0, min.z - 2.0]),
            max: section_at([max.x + 2.0, max.y, max.z + 2.0]),
        }
    }

    /// Only occupied keys, in absolute X and then packed Z/Y order.
    pub fn sections<T>(
        self,
        sections: &BTreeMap<SectionKey, T>,
        after: Option<SectionKey>,
    ) -> impl Iterator<Item = (&SectionKey, &T)> {
        let lower = after.map_or(Included(SectionKey(self.min.x, i64::MIN)), Excluded);
        let upper = SectionKey(self.max.x, i64::MAX);
        sections
            .range((lower, Included(upper)))
            .filter(move |(key, _)| {
                let pos = SectionPos::unpack(key.1);
                (self.min.y..=self.max.y).contains(&pos.y)
                    && (self.min.z..=self.max.z).contains(&pos.z)
            })
    }
}

type Entry = (JoinSeq, NonZeroU32);
pub type SectionCandidate = (SectionKey, Entry);

#[derive(Default)]
struct LevelSections {
    unfiled: BTreeSet<NonZeroU32>,
    occupied: BTreeMap<SectionKey, BTreeSet<Entry>>,
}

/// Rows stay in place across extraction/restoration; only admission, crossing
/// and retirement affect the spatial index.
#[derive(Default)]
pub(super) struct SpatialIndex(FxHashMap<DimensionKey, LevelSections>);

impl SpatialIndex {
    pub(super) fn unfile(&mut self, id: NonZeroU32, row: ResidentRow) {
        let level = self.0.entry(row.location.owner.dimension).or_default();
        if let Some(key) = row.section {
            remove_member(level, key, (row.entry, id));
        }
        level.unfiled.insert(id);
    }

    pub(super) fn retire(&mut self, id: NonZeroU32, row: ResidentRow) {
        let dimension = row.location.owner.dimension;
        if let Some(level) = self.0.get_mut(&dimension) {
            if let Some(key) = row.section {
                remove_member(level, key, (row.entry, id));
            } else {
                level.unfiled.remove(&id);
            }
            if level.occupied.is_empty() && level.unfiled.is_empty() {
                self.0.remove(&dimension);
            }
        }
    }

    pub(super) fn unfiled_after(
        &self,
        dimension: DimensionKey,
        after: Option<NonZeroU32>,
    ) -> Option<NonZeroU32> {
        let level = self.0.get(&dimension)?;
        level
            .unfiled
            .range((after.map_or(Unbounded, Excluded), Unbounded))
            .next()
            .copied()
    }

    pub(super) fn file(&mut self, id: NonZeroU32, row: &mut ResidentRow, at: [f64; 3]) {
        if let Some(level) = self.0.get_mut(&row.location.owner.dimension)
            && level.unfiled.remove(&id)
        {
            let key = SectionKey::at(at);
            level
                .occupied
                .entry(key)
                .or_default()
                .insert((row.entry, id));
            row.section = Some(key);
        }
    }

    pub(super) fn candidate_after(
        &self,
        dimension: DimensionKey,
        search: SectionSearch,
        after: Option<SectionCandidate>,
    ) -> Option<SectionCandidate> {
        let occupied = &self.0.get(&dimension)?.occupied;
        if let Some((key, entry)) = after
            && let Some(entries) = occupied.get(&key)
            && let Some(next) = entries.range((Excluded(entry), Unbounded)).next()
        {
            return Some((key, *next));
        }
        let (&key, entries) = search
            .sections(occupied, after.map(|(key, _)| key))
            .next()?;
        Some((key, *entries.first()?))
    }
}

fn remove_member(level: &mut LevelSections, key: SectionKey, entry: Entry) {
    if let Some(members) = level.occupied.get_mut(&key) {
        members.remove(&entry);
        if members.is_empty() {
            level.occupied.remove(&key);
        }
    }
}
