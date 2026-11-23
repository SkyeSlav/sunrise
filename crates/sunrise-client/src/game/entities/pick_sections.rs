//! Spatial section index matching vanilla `EntitySectionStorage` for crosshair picking.
use super::{aabb_extents, knot_bounds};
use crate::game::{Dim, Game, entity::Entity as _, general::general_type_catalog};
use std::collections::BTreeMap;
use sunrise_client::fast_hash::FxHashMap;
use sunrise_client::join_seq::{JoinSeq, Joined as _};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::SectionPos,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::game) enum PickKind {
    Mob,
    Boat,
    Cart,
    ArmorStand,
    Tnt,
    Falling,
    Projectile,
    Trident,
    Rocket,
    General,
    Painting,
    ItemFrame,
    Cushion,
    Interaction,
    RemoteGeneral,
    RemoteKnot,
    LocalKnot,
    Crystal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::game) struct PickMember {
    pub kind: PickKind,
    pub id: u32,
}

fn section_key([x, y, z]: [f64; 3]) -> i64 {
    SectionPos::new(
        sunrise_core::mth::floor(x) >> 4,
        sunrise_core::mth::floor(y) >> 4,
        sunrise_core::mth::floor(z) >> 4,
    )
    .pack()
}

fn hanging_center((lo, hi): ([f64; 3], [f64; 3])) -> [f64; 3] {
    [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ]
}

fn knot_position(block: [i32; 3]) -> [f64; 3] {
    let p = sunrise_client::mobs::leash::knot_state(block).position;
    [p.x, p.y, p.z]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PickFingerprint {
    epoch: Option<mod_api::WorldEpoch>,
    dim: Option<Dim>,
    is_remote: bool,
    mobs: (usize, Option<JoinSeq>),
    boats: (usize, Option<JoinSeq>),
    carts: (usize, Option<JoinSeq>),
    armor_stands: (usize, Option<JoinSeq>),
    tnts: (usize, Option<JoinSeq>),
    falling: (usize, Option<JoinSeq>),
    projectiles: (usize, Option<JoinSeq>),
    tridents: (usize, Option<JoinSeq>),
    rockets: (usize, Option<JoinSeq>),
    general: (usize, Option<JoinSeq>),
    paintings: (usize, Option<JoinSeq>),
    item_frames: (usize, Option<JoinSeq>),
    cushions: (usize, Option<JoinSeq>),
    interactions: usize,
    remote_general: (usize, Option<JoinSeq>),
    remote_knots: usize,
    local_knots: (usize, Option<JoinSeq>),
    crystals: (usize, Option<JoinSeq>),
}

impl PickFingerprint {
    fn same_level(self, other: Self) -> bool {
        self.epoch == other.epoch && self.dim == other.dim && self.is_remote == other.is_remote
    }

    fn of(game: &Game<'_>) -> Self {
        Self {
            epoch: game.native_lifecycle.epoch(),
            dim: Some(game.dim),
            is_remote: game.remote.is_some(),
            mobs: (game.mobs.len(), game.mobs.last().map(|e| e.join_seq())),
            boats: (game.boats.len(), game.boats.last().map(|e| e.join_seq())),
            carts: (game.carts.len(), game.carts.last().map(|e| e.join_seq())),
            armor_stands: (
                game.armor_stands.len(),
                game.armor_stands.last().map(|e| e.join_seq()),
            ),
            tnts: (game.tnts.len(), game.tnts.last().map(|e| e.join_seq())),
            falling: (
                game.falling.len(),
                game.falling.last().map(|e| e.join_seq()),
            ),
            projectiles: (
                game.creatures.projectiles.len(),
                game.creatures.projectiles.last().map(|e| e.join_seq()),
            ),
            tridents: (
                game.tridents.len(),
                game.tridents.last().map(|e| e.join_seq()),
            ),
            rockets: (
                game.rockets.len(),
                game.rockets.last().map(|e| e.join_seq()),
            ),
            general: (
                game.resident_entities.general.rows.len(),
                game.resident_entities
                    .general
                    .rows
                    .last()
                    .map(|e| e.join_seq()),
            ),
            paintings: (game.paintings.len(), game.paintings.last().map(|e| e.join)),
            item_frames: (
                game.item_frames.len(),
                game.item_frames.last().map(|e| e.join),
            ),
            cushions: (game.cushions.len(), game.cushions.last().map(|e| e.join)),
            interactions: game.interactions.len(),
            remote_general: game.remote.as_ref().map_or((0, None), |r| {
                (
                    r.general.rows.len(),
                    r.general.rows.values().next_back().map(|e| e.join),
                )
            }),
            remote_knots: game.remote_knots.len(),
            local_knots: (
                game.resident_entities.knots.len(),
                game.resident_entities.knots.last().map(|e| e.join),
            ),
            crystals: (game.crystals.len(), game.crystals.last().map(|e| e.join)),
        }
    }
}

/// Sparse section map matching `EntitySectionStorage`: keys are packed `SectionPos` values and
/// each section preserves entity entry order across spawns and section crossings.
#[derive(Default)]
pub(in crate::game) struct PickSectionStorage {
    sections: BTreeMap<i64, Vec<PickMember>>,
    by_id: FxHashMap<u32, (PickKind, i64)>,
    initialized: bool,
    fingerprint: PickFingerprint,
    scratch_removed: Vec<u32>,
    scratch_added: Vec<((u64, u32), PickMember, [f64; 3])>,
}

impl PickSectionStorage {
    pub(super) fn clear(&mut self) {
        self.sections.clear();
        self.by_id.clear();
        self.initialized = false;
    }

    pub(super) fn place(&mut self, member: PickMember, at: [f64; 3]) {
        self.remove(member.id);
        let key = section_key(at);
        self.sections.entry(key).or_default().push(member);
        self.by_id.insert(member.id, (member.kind, key));
    }

    pub(super) fn move_id(&mut self, id: u32, to: [f64; 3]) {
        let Some(&(kind, old_key)) = self.by_id.get(&id) else {
            return;
        };
        let new_key = section_key(to);
        if old_key == new_key {
            return;
        }
        let member = PickMember { kind, id };
        if let Some(bucket) = self.sections.get_mut(&old_key) {
            if let Some(pos) = bucket.iter().position(|&m| m == member) {
                bucket.remove(pos);
            }
            if bucket.is_empty() {
                self.sections.remove(&old_key);
            }
        }
        self.sections.entry(new_key).or_default().push(member);
        self.by_id.insert(id, (kind, new_key));
    }

    pub(super) fn remove(&mut self, id: u32) {
        let Some((kind, old_key)) = self.by_id.remove(&id) else {
            return;
        };
        let member = PickMember { kind, id };
        if let Some(bucket) = self.sections.get_mut(&old_key) {
            if let Some(pos) = bucket.iter().position(|&m| m == member) {
                bucket.remove(pos);
            }
            if bucket.is_empty() {
                self.sections.remove(&old_key);
            }
        }
    }

    pub(super) fn kind_of(&self, id: u32) -> Option<PickKind> {
        self.by_id.get(&id).map(|&(kind, _)| kind)
    }

    fn note_fingerprint(&mut self, game: &Game<'_>) {
        let fp = PickFingerprint::of(game);
        if !self.fingerprint.same_level(fp) {
            self.clear();
        }
        self.fingerprint = fp;
    }

    /// Reconciles only when the world context or container counts changed outside incremental hooks.
    fn sync_with(&mut self, game: &Game<'_>) {
        let fp = PickFingerprint::of(game);
        if self.initialized && self.fingerprint == fp {
            return;
        }
        // TransientEntitySectionManager.Callback.onMove already moved entities into
        // section-entry order, even if the client has not queried the level yet.
        if !self.fingerprint.same_level(fp) {
            self.clear();
        } else {
            self.scratch_removed.clear();
            for (&id, &(kind, _)) in &self.by_id {
                if game.pick_member_position_of_kind(kind, id).is_none() {
                    self.scratch_removed.push(id);
                }
            }
            for i in 0..self.scratch_removed.len() {
                let id = self.scratch_removed[i];
                self.remove(id);
            }
        }

        self.scratch_added.clear();
        let by_id = &self.by_id;
        let added = &mut self.scratch_added;
        let mut consider = |seq: u64, kind: PickKind, id: u32, pos: [f64; 3]| {
            if !by_id.contains_key(&id) {
                added.push(((seq, id), PickMember { kind, id }, pos));
            }
        };

        for e in game.mobs.iter() {
            let p = e.position();
            consider(e.join_seq().get(), PickKind::Mob, e.id(), [p.x, p.y, p.z]);
        }
        for e in game.boats.iter() {
            let p = e.position();
            consider(e.join_seq().get(), PickKind::Boat, e.id(), [p.x, p.y, p.z]);
        }
        for e in game.carts.iter() {
            let p = e.position();
            consider(e.join_seq().get(), PickKind::Cart, e.id(), [p.x, p.y, p.z]);
        }
        for e in game.armor_stands.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::ArmorStand,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for e in game.tnts.iter() {
            let p = e.position();
            consider(e.join_seq().get(), PickKind::Tnt, e.id(), [p.x, p.y, p.z]);
        }
        for e in game.falling.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::Falling,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for e in game.creatures.projectiles.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::Projectile,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for e in game.tridents.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::Trident,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for e in game.rockets.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::Rocket,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for e in game.resident_entities.general.rows.iter() {
            let p = e.position();
            consider(
                e.join_seq().get(),
                PickKind::General,
                e.id(),
                [p.x, p.y, p.z],
            );
        }
        for p in game.paintings.iter() {
            consider(
                p.join.get(),
                PickKind::Painting,
                p.id,
                hanging_center(p.bounds()),
            );
        }
        for f in game.item_frames.iter() {
            consider(
                f.join.get(),
                PickKind::ItemFrame,
                f.id,
                hanging_center(f.bounds()),
            );
        }
        for c in game.cushions.iter() {
            consider(
                c.join.get(),
                PickKind::Cushion,
                c.id,
                [c.position.x, c.position.y, c.position.z],
            );
        }
        for (&id, i) in &game.interactions {
            consider(
                i.join.get(),
                PickKind::Interaction,
                id as u32,
                [i.position.x, i.position.y, i.position.z],
            );
        }
        if let Some(remote) = &game.remote {
            for (&id, row) in &remote.general.rows {
                consider(
                    row.join.get(),
                    PickKind::RemoteGeneral,
                    id as u32,
                    row.position,
                );
            }
        }
        for (&id, &block) in &game.remote_knots {
            consider(u64::MAX, PickKind::RemoteKnot, id, knot_position(block));
        }
        for k in game.resident_entities.knots.iter() {
            consider(
                k.join.get(),
                PickKind::LocalKnot,
                k.id,
                knot_position(k.fence),
            );
        }
        for c in game.crystals.iter().filter(|c| c.id != 0) {
            consider(
                c.join.get(),
                PickKind::Crystal,
                c.id,
                [c.position.x, c.position.y, c.position.z],
            );
        }

        if self.scratch_added.len() > 1 {
            self.scratch_added.sort_by_key(|(key, _, _)| *key);
        }
        for i in 0..self.scratch_added.len() {
            let (_, member, pos) = self.scratch_added[i];
            self.place(member, pos);
        }
        self.scratch_added.clear();
        self.fingerprint = fp;
        self.initialized = true;
    }

    /// `EntitySectionStorage.forEachAccessibleNonEmptySection(bb, consumer)`.
    fn for_each_in_bounds(&self, search: Aabb, mut visit: impl FnMut(PickMember)) {
        let min = search.min_position();
        let max = search.max_position();
        if !min.x.is_finite()
            || !min.y.is_finite()
            || !min.z.is_finite()
            || !max.x.is_finite()
            || !max.y.is_finite()
            || !max.z.is_finite()
        {
            return;
        }
        let x_min = sunrise_core::mth::floor(min.x - 2.0) >> 4;
        let y_min = sunrise_core::mth::floor(min.y - 4.0) >> 4;
        let z_min = sunrise_core::mth::floor(min.z - 2.0) >> 4;
        let x_max = sunrise_core::mth::floor(max.x + 2.0) >> 4;
        let y_max = sunrise_core::mth::floor(max.y + 0.0) >> 4;
        let z_max = sunrise_core::mth::floor(max.z + 2.0) >> 4;
        if x_min > x_max || y_min > y_max || z_min > z_max {
            return;
        }

        for x in x_min..=x_max {
            let lo = SectionPos::new(x, 0, 0).pack();
            let hi = SectionPos::new(x, -1, -1).pack();
            for (&key, members) in self.sections.range(lo..=hi) {
                let pos = SectionPos::unpack(key);
                if pos.y >= y_min && pos.y <= y_max && pos.z >= z_min && pos.z <= z_max {
                    for &member in members {
                        visit(member);
                    }
                }
            }
        }
    }
}

impl Game<'_> {
    /// Files or replaces entity `id` at its current section in entry order.
    pub(in crate::game) fn pick_section_sync_id(&mut self, id: u32) {
        if let Some((kind, pos)) = self.locate_pick_member(id) {
            let mut storage = self.client_pick.sections.borrow_mut();
            storage.note_fingerprint(self);
            storage.place(PickMember { kind, id }, pos);
        }
    }

    /// Updates entity `id`'s section after a position write; no-op within the same section.
    pub(in crate::game) fn pick_section_move(&mut self, id: u32, to: [f64; 3]) {
        self.client_pick.sections.borrow_mut().move_id(id, to);
    }

    /// Updates entity `id`'s section from its live position without scanning unrelated families.
    pub(in crate::game) fn pick_section_sync_moved_id(&mut self, id: u32) {
        let kind = self.client_pick.sections.borrow().kind_of(id);
        let pos = match kind {
            Some(kind) => self.pick_member_position_of_kind(kind, id),
            None => self.locate_pick_member(id).map(|(_, pos)| pos),
        };
        if let Some(pos) = pos {
            self.client_pick.sections.borrow_mut().move_id(id, pos);
        }
    }

    /// Removes entity `id` from section storage immediately on discard/removal.
    pub(in crate::game) fn pick_section_remove(&mut self, id: u32) {
        let mut storage = self.client_pick.sections.borrow_mut();
        storage.note_fingerprint(self);
        storage.remove(id);
    }

    /// Clears section storage when changing or resetting a level.
    pub(in crate::game) fn pick_section_clear(&mut self) {
        self.client_pick.sections.borrow_mut().clear();
    }

    /// Updates sections for seated passengers after `position_riders` moves them.
    pub(in crate::game) fn sync_seated_pick_sections(&mut self) {
        let mut storage = self.client_pick.sections.borrow_mut();
        if !storage.initialized && storage.by_id.is_empty() {
            return;
        }
        for m in self.mobs.iter() {
            if m.extra.mount.is_some() {
                let p = m.position();
                storage.move_id(m.id(), [p.x, p.y, p.z]);
            }
        }
        for a in self.armor_stands.iter() {
            if a.vehicle.is_some() {
                let p = a.position();
                storage.move_id(a.id(), [p.x, p.y, p.z]);
            }
        }
        for (&id, i) in &self.interactions {
            if i.vehicle.is_some() {
                storage.move_id(id as u32, [i.position.x, i.position.y, i.position.z]);
            }
        }
        if let Some(remote) = &self.remote {
            for (&id, row) in &remote.general.rows {
                if remote.passenger_vehicle(id as u32).is_some() {
                    storage.move_id(id as u32, row.position);
                }
            }
        }
    }

    fn locate_pick_member(&self, id: u32) -> Option<(PickKind, [f64; 3])> {
        if let Some(i) = self.mob_index.find_ref(&self.mobs, id) {
            let p = self.mobs[i].position;
            return Some((PickKind::Mob, [p.x, p.y, p.z]));
        }
        if let Some(b) = self.boats.iter().find(|e| e.id() == id) {
            let p = b.position();
            return Some((PickKind::Boat, [p.x, p.y, p.z]));
        }
        if let Some(c) = self.carts.iter().find(|e| e.id() == id) {
            let p = c.position();
            return Some((PickKind::Cart, [p.x, p.y, p.z]));
        }
        if let Some(a) = self.armor_stands.iter().find(|e| e.id() == id) {
            let p = a.position();
            return Some((PickKind::ArmorStand, [p.x, p.y, p.z]));
        }
        if let Some(t) = self.tnts.iter().find(|e| e.id() == id) {
            let p = t.position();
            return Some((PickKind::Tnt, [p.x, p.y, p.z]));
        }
        if let Some(f) = self.falling.iter().find(|e| e.id() == id) {
            let p = f.position();
            return Some((PickKind::Falling, [p.x, p.y, p.z]));
        }
        if let Some(p) = self.creatures.projectiles.iter().find(|e| e.id() == id) {
            let pos = p.position();
            return Some((PickKind::Projectile, [pos.x, pos.y, pos.z]));
        }
        if let Some(t) = self.tridents.iter().find(|e| e.id() == id) {
            let p = t.position();
            return Some((PickKind::Trident, [p.x, p.y, p.z]));
        }
        if let Some(r) = self.rockets.iter().find(|e| e.id() == id) {
            let p = r.position();
            return Some((PickKind::Rocket, [p.x, p.y, p.z]));
        }
        if let Some(g) = self.resident_entities.general.row(id) {
            let p = g.position();
            return Some((PickKind::General, [p.x, p.y, p.z]));
        }
        if let Some(p) = self.paintings.iter().find(|e| e.id == id) {
            return Some((PickKind::Painting, hanging_center(p.bounds())));
        }
        if let Some(f) = self.item_frames.iter().find(|e| e.id == id) {
            return Some((PickKind::ItemFrame, hanging_center(f.bounds())));
        }
        if let Some(c) = self.cushions.iter().find(|e| e.id == id) {
            return Some((
                PickKind::Cushion,
                [c.position.x, c.position.y, c.position.z],
            ));
        }
        if let Some(i) = self.interactions.get(&(id as i32)) {
            return Some((
                PickKind::Interaction,
                [i.position.x, i.position.y, i.position.z],
            ));
        }
        if let Some(row) = self
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&(id as i32)))
        {
            return Some((PickKind::RemoteGeneral, row.position));
        }
        if let Some(&block) = self.remote_knots.get(&id) {
            return Some((PickKind::RemoteKnot, knot_position(block)));
        }
        if let Some(k) = self.resident_entities.knots.iter().find(|k| k.id == id) {
            return Some((PickKind::LocalKnot, knot_position(k.fence)));
        }
        if let Some(c) = self.crystals.iter().find(|c| c.id == id) {
            return Some((
                PickKind::Crystal,
                [c.position.x, c.position.y, c.position.z],
            ));
        }
        None
    }

    fn pick_member_position_of_kind(&self, kind: PickKind, id: u32) -> Option<[f64; 3]> {
        match kind {
            PickKind::Mob => self.mob_index.find_ref(&self.mobs, id).map(|i| {
                let p = self.mobs[i].position;
                [p.x, p.y, p.z]
            }),
            PickKind::Boat => self.boats.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Cart => self.carts.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::ArmorStand => self.armor_stands.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Tnt => self.tnts.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Falling => self.falling.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Projectile => self
                .creatures
                .projectiles
                .iter()
                .find(|e| e.id() == id)
                .map(|e| {
                    let p = e.position();
                    [p.x, p.y, p.z]
                }),
            PickKind::Trident => self.tridents.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Rocket => self.rockets.iter().find(|e| e.id() == id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::General => self.resident_entities.general.row(id).map(|e| {
                let p = e.position();
                [p.x, p.y, p.z]
            }),
            PickKind::Painting => self
                .paintings
                .iter()
                .find(|e| e.id == id)
                .map(|e| hanging_center(e.bounds())),
            PickKind::ItemFrame => self
                .item_frames
                .iter()
                .find(|e| e.id == id)
                .map(|e| hanging_center(e.bounds())),
            PickKind::Cushion => self
                .cushions
                .iter()
                .find(|e| e.id == id)
                .map(|e| [e.position.x, e.position.y, e.position.z]),
            PickKind::Interaction => self
                .interactions
                .get(&(id as i32))
                .map(|e| [e.position.x, e.position.y, e.position.z]),
            PickKind::RemoteGeneral => self
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))
                .map(|e| e.position),
            PickKind::RemoteKnot => self.remote_knots.get(&id).copied().map(knot_position),
            PickKind::LocalKnot => self
                .resident_entities
                .knots
                .iter()
                .find(|k| k.id == id)
                .map(|k| knot_position(k.fence)),
            PickKind::Crystal => self
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
        }
    }

    /// A member's id, box and pick radius; a client's copy of a non-mob entity takes the box of
    /// its `entity_size` answer.
    fn resolve_pick_member(&self, member: PickMember) -> Option<(u32, Aabb, f64)> {
        let id = member.id;
        match member.kind {
            PickKind::Mob => {
                let m = &self.mobs[self.mob_index.find_ref(&self.mobs, id)?];
                m.is_pickable().then(|| (m.id, m.bounds(), 0.0))
            }
            PickKind::Boat => self.boats.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Cart => self.carts.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::ArmorStand => self
                .armor_stands
                .iter()
                .find(|e| e.id == id)
                .filter(|e| e.is_pickable())
                .map(|e| {
                    (
                        e.id,
                        self.client_copy_box(e.id, e.position, e.bounds()),
                        0.0,
                    )
                }),
            PickKind::Tnt => self.tnts.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Falling => self.falling.iter().find(|e| e.id == id).map(|e| {
                (
                    e.id,
                    self.client_copy_box(e.id, e.position, e.bounds()),
                    0.0,
                )
            }),
            PickKind::Projectile => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .filter(|p| self.projectile_is_pickable(p))
                .map(|p| {
                    let bounds = self.client_copy_box(p.id, p.position, p.bounds());
                    (p.id, bounds, self.projectile_pick_radius(p))
                }),
            PickKind::Trident => self
                .tridents
                .iter()
                .find(|t| t.id == id)
                .filter(|t| self.trident_is_pickable(t))
                .map(|t| {
                    (
                        t.id,
                        self.client_copy_box(t.id, t.position, t.bounds()),
                        1.0,
                    )
                }),
            PickKind::Rocket => self
                .rockets
                .iter()
                .find(|r| r.id == id)
                .filter(|_| self.rocket_is_pickable())
                .map(|r| {
                    (
                        r.id,
                        self.client_copy_box(r.id, r.position, r.bounds()),
                        1.0,
                    )
                }),
            PickKind::General => self
                .resident_entities
                .general
                .row(id)
                .filter(|r| r.is_pickable())
                .map(|r| (r.id, r.bounds(), 0.0)),
            PickKind::Painting => self
                .paintings
                .iter()
                .find(|p| p.id == id)
                .map(|p| (p.id, aabb_extents(p.bounds()), 0.0)),
            PickKind::ItemFrame => self
                .item_frames
                .iter()
                .find(|f| f.id == id)
                .map(|f| (f.id, aabb_extents(f.bounds()), 0.0)),
            PickKind::Cushion => self
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| (c.id, aabb_extents(c.bounds()), 0.0)),
            PickKind::Interaction => self
                .interactions
                .get(&(id as i32))
                .map(|i| (id, self.client_copy_box(id, i.position, i.bounds()), 0.0)),
            PickKind::RemoteGeneral => self
                .remote
                .as_ref()?
                .general
                .rows
                .get(&(id as i32))
                .filter(|row| general_type_catalog().pickable(row.type_slot))
                .map(|row| (id, row.bounds(), 0.0)),
            PickKind::RemoteKnot => self
                .remote_knots
                .get(&id)
                .copied()
                .map(|block| (id, knot_bounds(block), 0.0)),
            PickKind::LocalKnot => self
                .resident_entities
                .knots
                .iter()
                .find(|k| k.id == id)
                .map(|k| (k.id, knot_bounds(k.fence), 0.0)),
            PickKind::Crystal => self.crystals.iter().find(|c| c.id == id).map(|c| {
                (
                    c.id,
                    self.client_copy_box(c.id, c.position, c.bounds()),
                    0.0,
                )
            }),
        }
    }

    fn live_entity_bounds_for_pick(&self, id: u32) -> Option<Aabb> {
        if let Some(i) = self.mob_index.find_ref(&self.mobs, id) {
            return Some(self.mobs[i].bounds());
        }
        if let Some(a) = self.armor_stands.iter().find(|e| e.id == id) {
            return Some(a.bounds());
        }
        if let Some(b) = self.boats.iter().find(|e| e.id == id) {
            return Some(b.bounds());
        }
        if let Some(c) = self.carts.iter().find(|e| e.id == id) {
            return Some(c.bounds());
        }
        if let Some(t) = self.tnts.iter().find(|e| e.id == id) {
            return Some(t.bounds());
        }
        if let Some(f) = self.falling.iter().find(|e| e.id == id) {
            return Some(f.bounds());
        }
        if let Some(p) = self.paintings.iter().find(|e| e.id == id) {
            return Some(aabb_extents(p.bounds()));
        }
        if let Some(f) = self.item_frames.iter().find(|e| e.id == id) {
            return Some(aabb_extents(f.bounds()));
        }
        if let Some(c) = self.cushions.iter().find(|e| e.id == id) {
            return Some(aabb_extents(c.bounds()));
        }
        if let Some(i) = self.interactions.get(&(id as i32)) {
            return Some(i.bounds());
        }
        if let Some(p) = self.creatures.projectiles.iter().find(|e| e.id == id) {
            return Some(p.bounds());
        }
        if let Some(t) = self.tridents.iter().find(|e| e.id == id) {
            return Some(t.bounds());
        }
        if let Some(r) = self.rockets.iter().find(|e| e.id == id) {
            return Some(r.bounds());
        }
        if let Some(c) = self.crystals.iter().find(|e| e.id == id) {
            return Some(c.bounds());
        }
        if let Some(d) = self
            .dragon
            .as_ref()
            .filter(|d| (d.id != 0 || self.remote.is_some()) && d.id == id)
        {
            let p = d.position;
            return Some(Aabb::new(
                Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
                Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
            ));
        }
        if let Some(row) = self
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&(id as i32)))
        {
            return Some(row.bounds());
        }
        if let Some(&block) = self.remote_knots.get(&id) {
            return Some(knot_bounds(block));
        }
        if let Some(k) = self.resident_entities.knots.iter().find(|k| k.id == id) {
            return Some(knot_bounds(k.fence));
        }
        None
    }

    /// `cameraEntity.getBoundingBox()` at the camera entity's current position.
    pub(super) fn pick_camera_bounds(&self) -> Option<Aabb> {
        if let Some(r) = self.remote.as_ref()
            && let Some(cam_id) = r.camera
        {
            if let Some(detached) = r
                .detached_camera
                .as_deref()
                .filter(|d| d.entity_id == cam_id as u32)
            {
                let p = detached.camera.position;
                let v = Vec3::new(p[0], p[1], p[2]);
                return Some(Aabb::new(v, v));
            }
            if cam_id == r.entity_id {
                return self.player.as_ref().map(|p| p.bounds());
            }
            return self.live_entity_bounds_for_pick(cam_id as u32);
        }
        self.player.as_ref().map(|p| p.bounds())
    }

    /// `Level.getEntities(cameraEntity, search_box, EntitySelector.CAN_BE_PICKED)`:
    /// visits ordinary entities in `EntitySectionStorage` order (filtered by raw box),
    /// then appends live dragon parts (unless spectating that dragon).
    pub(in crate::game) fn for_each_pickable_in_bounds(
        &self,
        search_box: Aabb,
        mut visit: impl FnMut(u32, Aabb, f64),
    ) {
        {
            self.client_pick.sections.borrow_mut().sync_with(self);
        }
        {
            let storage = self.client_pick.sections.borrow();
            storage.for_each_in_bounds(search_box, |member| {
                if let Some((id, raw_bb, radius)) = self.resolve_pick_member(member)
                    && !self.is_pick_camera(id)
                    && raw_bb.intersects(search_box)
                {
                    visit(id, raw_bb, radius);
                }
            });
        }
        if let Some(d) = self.dragon.as_ref().filter(|d| !self.is_pick_camera(d.id)) {
            let has_wire_id = d.id != 0 || self.remote.is_some();
            for i in 0..8 {
                let part_id = if has_wire_id {
                    crate::game::dragon::dragon_part_id(d.id, i)
                } else {
                    crate::game::dragon::SPECIAL + i as u32
                };
                let part_bb = d.part_geometry(i).1;
                if !self.is_pick_camera(part_id) && part_bb.intersects(search_box) {
                    visit(part_id, part_bb, 0.0);
                }
            }
        }
    }
}
