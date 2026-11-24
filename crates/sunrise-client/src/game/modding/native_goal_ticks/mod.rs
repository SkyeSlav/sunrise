//! The mod goal context of a mob's tick (arismoko/horizon#1077): the reads a
//! goal makes through `GoalContext::host` answer the ticking mob itself and
//! peer mobs from the tick's disjoint live loan, while players, the dragon,
//! non-mob entities, and every level/world/block-entity read forward straight
//! to the owning game host. The writes a goal makes apply to the mob in hand
//! through the engine's own paths (`nav_move_to`, `ai.target`), behind the
//! same input boundaries the mob handles take, and the write handles a
//! `ServerHost` takes do not exist here.
use super::Game;
use crate::mobs::{
    self, Mob, MobWorld, Who,
    part_goals::{BehaviorOp, GoalOp},
};
use mod_api::{
    BehaviorCxBackend, EntityRef, EntityTags, GoalCxBackend, Host, LivingRef, MobRef, PlayerRef,
    RandomSource,
    content::crash::{ModCall, enter_native_mod},
};
use sunrise_core::geometry::{Axis, Vec3};
use sunrise_core::random::RandomSource as EngineRandom;

#[cfg(test)]
mod interception_tests;
mod live_backend;
mod live_entity;
mod live_living;
mod live_merchant;
mod live_mob;
#[cfg(test)]
pub(in crate::game) mod live_read_tests;
mod live_support;
mod live_vehicle;
#[cfg(test)]
mod redirect_tests;

/// What a mod goal runs with: the mob in hand, the tick's `MobWorld` (the
/// [`mobs::part_goals::GoalHost`], the snapshot and the player view ride on
/// it) and the game host behind the tick, for the reads that are the level's
/// own. The writes apply to the mob itself.
pub(super) struct GoalCx<'a, 'w> {
    m: &'a mut Mob,
    w: &'a MobWorld<'w>,
    host: &'a dyn Host,
}

/// What a handle names in the goal view when it is not served directly by the
/// owning `Game` host: the ticking mob itself, another mob in the tick's
/// disjoint live loan, or a synthetic `PlayerView` supplied without a backing
/// `Game` player (isolated unit tests). Real players, the dragon and non-mob
/// entities stay in `Game` and return `None` from [`GoalCx::resolve`] so
/// [`GoalCx::is_game_entity`] delegates every read directly to `self.host`.
enum Resolved<'a> {
    Own,
    SyntheticPlayer,
    Live(&'a Mob),
    /// Removed, held out, or otherwise authoritatively unavailable over the
    /// game host behind the tick.
    Unavailable,
}

impl GoalCx<'_, '_> {
    /// Resolve `entity` in the tick's live mob loan. Removed and held-out
    /// entities are authoritatively unavailable; `None` lets `is_game_entity`
    /// delegate real players, the dragon, non-mob entities and cross-level
    /// entities directly to `self.host`.
    fn resolve(&self, entity: EntityRef) -> Option<Resolved<'_>> {
        let host = self.w.goal_host?;
        if host.epoch() != Some(entity.epoch()) {
            return None;
        }
        let id = entity.id();
        if host.is_held_out(id) {
            return Some(Resolved::Unavailable);
        }
        if id == self.m.id {
            return Some(if self.m.removed {
                Resolved::Unavailable
            } else {
                Resolved::Own
            });
        }
        if let Some(mobs) = self.w.live_mobs {
            if let Some(mob) = mobs.find(id) {
                return Some(if mob.removed {
                    Resolved::Unavailable
                } else {
                    Resolved::Live(mob)
                });
            }
        } else if let Some(seen) = self.w.seen(id)
            && seen.removed
        {
            return Some(Resolved::Unavailable);
        }
        // Real Game entities (players, the dragon, armor stands, items, ...)
        // stay in Game and delegate straight to `self.host`.
        if self.host.entity(id) == Some(entity) {
            return None;
        }
        // Isolated unit tests may lend a synthetic `PlayerView` without a
        // corresponding player entity in `Game`.
        if self.w.selected_player().is_some()
            && self
                .w
                .selected_player_identity()
                .is_some_and(|(_, player)| player == id)
        {
            return Some(Resolved::SyntheticPlayer);
        }
        None
    }

    /// Whether `entity` names the player currently selected for this mob tick.
    fn is_selected_player(&self, entity: EntityRef) -> bool {
        self.w
            .goal_host
            .is_some_and(|host| host.epoch() == Some(entity.epoch()))
            && self.w.selected_player().is_some()
            && self
                .w
                .selected_player_identity()
                .is_some_and(|(_, player)| player == entity.id())
    }

    // --- the entity lookups the generated Host impl forwards to ----------

    /// The ticking mob's own handle, as both the goal and the behavior
    /// context build it: the goal host's epoch around the mob in hand.
    fn mob_handle(&self) -> MobRef {
        let host = self
            .w
            .goal_host
            .expect("a goal context is built only where a goal host is");
        let epoch = host.epoch().expect("a goal host is built only in a world");
        MobRef::new(EntityRef::new(epoch, self.m.id))
    }

    /// `Host::entity`: the id resolves in the tick's live mob loan or the
    /// owning game host.
    pub(crate) fn lookup_entity(&self, id: u32) -> Option<EntityRef> {
        let host = self.w.goal_host?;
        let epoch = host.epoch()?;
        let entity = EntityRef::new(epoch, id);
        match self.resolve(entity) {
            Some(Resolved::Own | Resolved::SyntheticPlayer | Resolved::Live(_)) => Some(entity),
            // An unavailable entity is the view's own answer: never ask the host.
            Some(Resolved::Unavailable) => None,
            None => self.host.entity(id),
        }
    }

    /// `Host::living`: the handle resolves in the tick's live mob loan or the
    /// owning game host.
    pub(crate) fn lookup_living(&self, entity: EntityRef) -> Option<LivingRef> {
        match self.resolve(entity) {
            Some(Resolved::Unavailable) => None,
            Some(_) => Some(LivingRef::new(entity)),
            None => self.host.living(entity),
        }
    }

    /// `Host::mob`: the handle resolves in the tick's live mob loan or the
    /// owning game host as a mob, never as a player.
    pub(crate) fn lookup_mob(&self, entity: EntityRef) -> Option<MobRef> {
        match self.resolve(entity) {
            Some(Resolved::SyntheticPlayer | Resolved::Unavailable) => None,
            Some(_) => Some(MobRef::new(entity)),
            None => self.host.mob(entity),
        }
    }

    /// `Host::player`: resolve any online player, including a synthetic test
    /// player view, by UUID.
    pub(crate) fn lookup_player(&self, uuid: u128) -> Option<PlayerRef> {
        self.host.player(uuid).or_else(|| {
            let (uuid_seen, id) = self.w.selected_player_identity()?;
            let epoch = self.w.goal_host?.epoch()?;
            (self.w.selected_player().is_some()
                && uuid == sunrise_client::lan::playerdata::ints_uuid(uuid_seen))
            .then(|| mod_api::__private::game::player_ref(EntityRef::new(epoch, id), uuid))
        })
    }

    /// `Host::item_entity`: items remain in the game during mob ticks.
    pub(crate) fn lookup_item_entity(&self, entity: EntityRef) -> Option<mod_api::ItemEntityRef> {
        self.host.item_entity(entity)
    }

    /// `Host::block_entity`: block entities live in the world's chunks, which
    /// the tick does not take out — the game host answers for them.
    pub(crate) fn lookup_block_entity(
        &self,
        level: mod_api::LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::BlockEntityRef> {
        self.host.block_entity(level, pos)
    }

    /// The entity's position, or None where unresolvable.
    fn position(&self, entity: EntityRef) -> Option<sunrise_core::geometry::Vec3> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.position),
            Resolved::Live(mob) => Some(mob.position),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.position),
            Resolved::Unavailable => None,
        }
    }

    // --- the live-loan reads the generated view forwards to ----------------

    fn r_entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        self.position(entity).map(|p| [p.x, p.y, p.z])
    }

    fn r_entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]> {
        self.position(entity)
            .map(|p| [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32])
    }

    fn r_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.velocity),
            Resolved::Live(mob) => Some(mob.velocity),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.velocity),
            Resolved::Unavailable => None,
        }
        .map(|v| [v.x, v.y, v.z])
    }

    fn r_entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        match self.resolve(entity)? {
            Resolved::Own => Some([self.m.yaw, self.m.head_pitch]),
            Resolved::Live(mob) => Some([mob.yaw, mob.head_pitch]),
            _ => None,
        }
    }

    fn r_entity_head_yaw(&self, entity: EntityRef) -> Option<f32> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.head_yaw),
            Resolved::Live(mob) => Some(mob.head_yaw),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.yaw),
            Resolved::Unavailable => None,
        }
    }

    fn r_entity_body_yaw(&self, entity: EntityRef) -> Option<f32> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.body_yaw),
            Resolved::Live(mob) => Some(mob.body_yaw),
            _ => None,
        }
    }

    fn r_entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]> {
        let bounds = match self.resolve(entity)? {
            Resolved::Own => self.m.bounds(),
            Resolved::Live(mob) => mob.bounds(),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.bounds)?,
            Resolved::Unavailable => return None,
        };
        let axes = [Axis::X, Axis::Y, Axis::Z];
        Some([axes.map(|a| bounds.min(a)), axes.map(|a| bounds.max(a))])
    }

    fn r_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.eye_y() - self.m.position.y),
            Resolved::Live(mob) => Some(mob.eye_y() - mob.position.y),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.eye[1] - p.position.y),
            Resolved::Unavailable => None,
        }
    }

    fn r_entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        match self.resolve(entity)? {
            Resolved::Own => {
                let y = self.m.eye_y();
                Some([self.m.position.x, y, self.m.position.z])
            }
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.eye),
            Resolved::Live(mob) => Some([mob.position.x, mob.eye_y(), mob.position.z]),
            Resolved::Unavailable => None,
        }
    }

    fn r_entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64> {
        self.position(entity).map(|p| {
            (p.x - pos[0]).mul_add(p.x - pos[0], 0.0)
                + (p.y - pos[1]).mul_add(p.y - pos[1], 0.0)
                + (p.z - pos[2]).mul_add(p.z - pos[2], 0.0)
        })
    }

    fn r_entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        match self.resolve(entity)? {
            Resolved::Own => Some(self.m.ground.on_ground),
            Resolved::Live(mob) => Some(mob.ground.on_ground),
            _ => None,
        }
    }

    fn r_entity_kind(&self, entity: EntityRef) -> Option<mod_api::EntityTypeKey> {
        match self.resolve(entity)? {
            Resolved::Own => mod_api::__private::resolved_entity_type_key(&self.m.kind.id),
            Resolved::Live(mob) => mod_api::__private::resolved_entity_type_key(&mob.kind.id),
            _ => None,
        }
    }

    fn r_living_health(&self, living: LivingRef) -> Option<f32> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.health),
            Resolved::Live(mob) => Some(mob.health),
            _ => None,
        }
    }

    fn r_living_max_health(&self, living: LivingRef) -> Option<f32> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.attrs.max_health),
            Resolved::Live(mob) => Some(mob.attrs.max_health),
            _ => None,
        }
    }

    fn r_living_alive(&self, living: LivingRef) -> Option<bool> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m.alive()),
            Resolved::Live(mob) => Some(mob.alive()),
            Resolved::SyntheticPlayer => self.w.selected_player().map(|p| p.alive),
            Resolved::Unavailable => None,
        }
    }
}

/// The ticking mob's own random, as its goals draw from it: each draw is the engine's, in the
/// mob's order.
impl RandomSource for GoalCx<'_, '_> {
    fn next_int(&mut self) -> i32 {
        EngineRandom::next_int(&mut self.m.random)
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        EngineRandom::next_int_bound(&mut self.m.random, bound).unwrap_or(0)
    }
    fn next_long(&mut self) -> i64 {
        EngineRandom::next_long(&mut self.m.random)
    }
    fn next_boolean(&mut self) -> bool {
        EngineRandom::next_boolean(&mut self.m.random)
    }
    fn next_float(&mut self) -> f32 {
        EngineRandom::next_float(&mut self.m.random)
    }
    fn next_double(&mut self) -> f64 {
        EngineRandom::next_double(&mut self.m.random)
    }
    fn next_gaussian(&mut self) -> f64 {
        EngineRandom::next_gaussian(&mut self.m.random)
    }
}

impl GoalCxBackend for GoalCx<'_, '_> {
    fn host(&self) -> &dyn Host {
        self
    }

    fn mob(&self) -> MobRef {
        self.mob_handle()
    }

    fn random(&mut self) -> &mut dyn RandomSource {
        self
    }

    fn move_to(&mut self, at: [f64; 3], speed: f64) -> bool {
        // The navigation handle's own boundary: numbers the pathfinder does
        // not take refuse here as they refuse there.
        if !super::native_mob_navigation::navigation_move_accepts(at, speed) {
            return false;
        }
        self.m.nav_move_to(self.w, at, speed)
    }

    fn move_to_path(
        &mut self,
        path: &[mod_api::PathNode],
        speed: f64,
    ) -> Result<(), mod_api::MoveToPathError> {
        // The handle's own boundary, the same one `MobRef::move_to_path` takes.
        if !super::native_mob_navigation::supplied_path_accepts(path, speed) {
            return Err(mod_api::MoveToPathError::NotFinite);
        }
        let followed =
            self.m
                .nav_move_to_supplied(self.w, path.iter().map(|n| [n.x, n.y, n.z]), speed);
        super::native_mob_navigation::supplied_path_refusal(followed)
    }

    fn stop_navigation(&mut self) {
        self.m.nav_stop();
    }

    fn look_at(&mut self, at: [f64; 3]) {
        // The look control's boundary, as the navigation handle takes it.
        if super::native_mob_navigation::navigation_look_accepts(at) {
            self.m.look_at(at);
        }
    }

    fn set_target(&mut self, target: Option<LivingRef>) -> bool {
        // Brain-backed species refuse `setTarget` before the event, matching
        // `MobRef::set_target`.
        if self.m.refuses_set_target() {
            return false;
        }
        let mut prev_selected = None;
        let who = match target {
            None => None,
            Some(living) => {
                let entity = living.entity();
                let Some(host) = self.w.goal_host else {
                    return false;
                };
                if host.epoch() != Some(entity.epoch()) {
                    return false;
                }
                if host.is_fake_player(entity) {
                    return false;
                }
                match self.resolve(entity) {
                    Some(Resolved::Own) => Some(Who::Mob(self.m.id)),
                    Some(Resolved::Live(mob)) => Some(Who::Mob(mob.id)),
                    Some(Resolved::SyntheticPlayer) => Some(Who::Player),
                    Some(Resolved::Unavailable) => return false,
                    None if self.is_selected_player(entity) => Some(Who::Player),
                    None => {
                        if let Some(selection) = self.w.player_selection
                            && let Some(player) = selection.get(entity.id())
                        {
                            let valid = self.m.valid_target(
                                self.w,
                                &mobs::goals::TargetFacts::Player {
                                    alive: player.view.alive,
                                    invulnerable: player.view.invulnerable,
                                    creative: player.view.creative,
                                    spectator: player.view.spectator,
                                    uuid: player.view.uuid,
                                },
                            );
                            if !valid {
                                None
                            } else {
                                prev_selected = selection.selected().map(|p| p.entity_id);
                                if selection.select(entity.id()) {
                                    Some(Who::Player)
                                } else {
                                    return false;
                                }
                            }
                        } else if self.host.living(entity).is_some()
                            && !host.is_unbindable_player(entity)
                        {
                            // A live non-mob/non-player living entity (armor stand,
                            // dragon) fails `Mob.asValidTarget` and normalizes to None.
                            None
                        } else {
                            return false;
                        }
                    }
                }
            }
        };
        let change = self.m.set_ai_target(self.w, who);
        if matches!(change, mobs::TargetChange::Keep)
            && let (Some(selection), Some(prev)) = (self.w.player_selection, prev_selected)
        {
            selection.select(prev);
        }
        matches!(change, mobs::TargetChange::Set(_))
    }

    fn jump(&mut self) {
        // `JumpControl.jump`: the wish the control consumes on the mob's next
        // own tick (it assigns `jumping` from `jump` there), as the mob jump
        // handle writes it.
        self.m.ai.jump = true;
    }

    fn delta_movement(&self) -> [f64; 3] {
        let v = self.m.velocity;
        [v.x, v.y, v.z]
    }

    fn set_delta_movement(&mut self, velocity: [f64; 3]) -> bool {
        // `Entity.setDeltaMovement`: a vector that is not finite is not
        // stored. No sync is forced: as for vanilla's goals, the tracker
        // sends the motion on its own schedule.
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let [x, y, z] = velocity;
        self.m.velocity = Vec3::new(x, y, z);
        true
    }

    fn add_delta_movement(&mut self, delta: [f64; 3]) -> bool {
        // `Entity.addDeltaMovement`: a delta that is not finite is not
        // added, and `setDeltaMovement` refuses a sum that overflows.
        let v = self.m.velocity;
        let sum = [v.x + delta[0], v.y + delta[1], v.z + delta[2]];
        delta.iter().all(|d| d.is_finite()) && self.set_delta_movement(sum)
    }
}

impl BehaviorCxBackend for GoalCx<'_, '_> {
    fn host(&self) -> &dyn Host {
        self
    }

    fn mob(&self) -> MobRef {
        self.mob_handle()
    }

    fn walk_target(&self) -> Option<mod_api::BrainWalkTarget> {
        mobs::mod_behaviors::walk_target(self.m, self.w)
    }

    fn look_target(&self) -> Option<[f64; 3]> {
        mobs::mod_behaviors::look_target(self.m, self.w)
    }

    fn attack_target(&self) -> Option<LivingRef> {
        // The brain's own target read through the engine's path, the handle
        // resolved in the tick's live view.
        let host = self.w.goal_host?;
        let epoch = host.epoch()?;
        if self.m.species() == mobs::Species::Warden {
            let id = mobs::warden::brain_target_id(self.m)?;
            return self.lookup_living(EntityRef::new(epoch, id));
        }
        let id = match mobs::ai::brain_target(self.m)?? {
            Who::Mob(id) => id,
            Who::Player => self.w.selected_player_identity()?.1,
        };
        self.lookup_living(EntityRef::new(epoch, id))
    }

    fn set_brain_target(&mut self, target: Option<LivingRef>) -> bool {
        // The write handle's own setter rules, over the tick's live view:
        // the mob itself, the player it sees, or another live peer mob.
        let who = match target {
            None => None,
            Some(living) => {
                let entity = living.entity();
                if self.w.goal_host.is_some_and(|h| h.is_fake_player(entity)) {
                    return false;
                }
                match self.resolve(entity) {
                    Some(Resolved::Own) => Some(Who::Mob(self.m.id)),
                    Some(Resolved::Live(mob)) => Some(Who::Mob(mob.id)),
                    Some(Resolved::SyntheticPlayer) => Some(Who::Player),
                    Some(Resolved::Unavailable) => return false,
                    None if self.is_selected_player(entity) => Some(Who::Player),
                    None => return false,
                }
            }
        };
        mobs::ai::set_brain_target(self.m, who, target.map(|t| t.entity().id()))
    }

    fn set_walk_target(&mut self, at: [i32; 3], speed: f64, close: i32) {
        mobs::mod_behaviors::set_walk_target(self.m, at, speed, close)
    }
}

impl mobs::part_goals::GoalHost for Game<'_> {
    fn epoch(&self) -> Option<mod_api::WorldEpoch> {
        self.native_lifecycle.epoch()
    }

    fn is_held_out(&self, id: u32) -> bool {
        self.held_out.contains(&id)
    }

    fn is_fake_player(&self, entity: EntityRef) -> bool {
        matches!(
            self.native_entity_target(entity),
            Some(t @ (super::commands::Target::Player | super::commands::Target::Guest(_)))
                if self.native_player_facts(t).is_none()
        )
    }

    fn is_unbindable_player(&self, entity: EntityRef) -> bool {
        matches!(
            self.native_entity_target(entity),
            Some(t @ (super::commands::Target::Player | super::commands::Target::Guest(_)))
                if !self.native_mob_can_bind_player(t)
        )
    }

    fn merchant_is_live(&self, mob: &Mob) -> bool {
        self.is_logical_server() && !mob.removed
    }

    fn merchant_offers_lent(&self, mob: &Mob) -> Option<Vec<mod_api::MerchantOffer>> {
        use super::api_values::TryIntoApi;
        self.merchant_read_lent(mob, |offers, _, _| offers.try_into_api().ok())
    }

    fn merchant_trading_player_lent(&self, mob: &Mob) -> Option<mod_api::PlayerRef> {
        let conn = self.lan_trade_conn(mob.id)?;
        self.player(self.lan_trade_player(conn)?)
    }

    fn merchant_trade_state_lent(&self, mob: &Mob) -> (i32, i32) {
        self.merchant_read_lent(mob, |_, xp, level| (xp, level))
    }

    fn visit_player_passenger_seats(
        &self,
        vehicle: u32,
        visit: &mut dyn FnMut(i64, u32) -> bool,
    ) -> bool {
        self.any_player_passenger_seat(vehicle, visit)
    }

    fn next_passenger_seat(&self, vehicle: u32, after: Option<u32>) -> Option<u32> {
        Game::next_passenger_seat(self, vehicle, after)
    }

    fn passenger_vehicle(&self, rider: u32) -> Option<u32> {
        self.seated_passenger_vehicle(rider)
    }

    fn mob_dismount_location(
        &self,
        vehicle: Option<&Mob>,
        vehicle_id: u32,
        rider: Option<&Mob>,
        passenger: EntityRef,
    ) -> Option<[f64; 3]> {
        self.scoped_mob_dismount_location(vehicle, vehicle_id, rider, passenger)
    }

    fn mob_player_controls(&self, mob: &Mob, player: u32) -> bool {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return false;
        };
        let Some(target) = self.native_entity_target(EntityRef::new(epoch, player)) else {
            return false;
        };
        matches!(
            target,
            super::commands::Target::Player | super::commands::Target::Guest(_)
        ) && self.native_player_controls(mob, target)
    }

    fn mob_tags(&self, mob: &Mob) -> Option<EntityTags<'_>> {
        if self.remote.is_some() {
            return None;
        }
        Some(self.saved_entity_tags(super::commands::Target::Mob(mob.id)))
    }

    fn mob_name(&self, mob: &Mob, actor: &Mob, world: &MobWorld<'_>) -> String {
        let name = match self.mob_team_scoped(mob, actor, world) {
            Some(team) => team.formatted_name(mob.command_name()),
            None => mob.command_name(),
        };
        name.plain()
    }

    fn mob_added_to_level(&self, mob: &Mob) -> bool {
        self.persist
            .live
            .binary_search_by_key(&mob.id, |entry| entry.0)
            .is_ok()
    }

    fn mob_portal_cooldown(&self, mob: &Mob) -> i32 {
        self.portals.entities.get(&mob.id).map_or(0, |p| p.cooldown)
    }

    fn mob_invulnerable_to_damage(&self, mob: &Mob, damage_type: &str) -> bool {
        self.mob_query_invulnerable(mob, damage_type)
    }

    fn mob_can_use_portal(&self, mob: &Mob) -> bool {
        self.mob_portal_eligible(mob)
    }

    fn mob_is_climbing(&self, mob: &Mob) -> bool {
        self.native_mob_is_climbing(mob)
    }

    fn mob_attacker_elsewhere(&self, attacker: u32) -> Option<EntityRef> {
        self.credited_elsewhere(attacker)
    }

    fn mob_in_other_level(&self, entity: EntityRef) -> bool {
        self.native_lifecycle.epoch() == Some(entity.epoch())
            && Game::mob_in_other_level(self, entity.id()).is_some()
    }

    fn mob_player_credit(&self, mob: &Mob) -> Option<Option<u32>> {
        let credit = *self.last_hurt_by_player.get(&mob.id)?;
        Some(
            self.resolve_player_credit(credit)
                .and_then(|conn| self.player_entity_id_by_connection(conn))
                .and_then(|id| u32::try_from(id).ok()),
        )
    }

    fn mob_target(&self, mob: &Mob, actor: &Mob, world: &MobWorld<'_>) -> Option<LivingRef> {
        if mob.species() == mobs::Species::Warden {
            let (id, _) = self.warden_attack_target(mob, Some((actor, world)))?;
            return Some(LivingRef::new(EntityRef::new(
                self.native_lifecycle.epoch()?,
                id,
            )));
        }
        let target = mobs::ai::brain_target(mob).unwrap_or(mob.ai.target)?;
        match target {
            Who::Player => {
                // Preserve the game's per-mob guest binding: the selected
                // player for this tick need not be this mob's own target. A
                // listener's redirect, however, already changed the tick's
                // selection while the persisted binding is still borrowed.
                let redirected = world
                    .player_selection
                    .filter(|selection| mob.id == actor.id && selection.has_switched())
                    .and_then(|selection| selection.selected())
                    .map(|player| player.entity_id);
                let id = match redirected {
                    Some(id) => id,
                    None => {
                        let host = self.lan.as_ref()?;
                        let conn = host.mob_players.get(&mob.id)?;
                        u32::try_from(host.hub.guest(*conn)?.entity_id).ok()?
                    }
                };
                let entity = self.entity(id)?;
                self.native_mob_valid_target(mob, self.native_entity_target(entity)?)
                    .then(|| self.living(entity))?
            }
            Who::Mob(id) => {
                // Mob tick temporarily lends the list, so the regular game
                // lookup cannot see it. The disjoint loan holds live mobs.
                let other = if id == mob.id {
                    mob
                } else {
                    world.live_mobs?.find(id)?
                };
                if other.removed {
                    return None;
                }
                let valid = mob.valid_target(
                    world,
                    &mobs::goals::TargetFacts::Mob {
                        id,
                        species: other.species(),
                        alive: other.alive(),
                        invulnerable: other.invulnerable_flag,
                        baby: other.is_baby(),
                    },
                );
                let epoch = self.native_lifecycle.epoch()?;
                valid.then(|| LivingRef::new(EntityRef::new(epoch, id)))
            }
        }
    }

    fn mob_on_block(&self, mob: &Mob) -> [i32; 3] {
        super::native_entity_motion::on_block_position(mob.ground.supporting_block, mob.position)
    }

    fn entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        entity_mob: Option<&Mob>,
        source: EntityRef,
        source_mob: Option<&Mob>,
        actor: &Mob,
        world: &MobWorld<'_>,
    ) -> Option<bool> {
        let own = match entity_mob {
            Some(mob) => self.mob_team_scoped(mob, actor, world),
            None => self.native_target_team(self.native_entity_target(entity)?)?,
        };
        let other = match source_mob {
            Some(mob) => self.mob_team_scoped(mob, actor, world),
            None => self.native_target_team(self.native_entity_target(source)?)?,
        };
        Some(sunrise_client::scoreboard::teams_allow_damage(own, other))
    }

    fn brain_times(&self) -> (i64, i64) {
        (self.world.game_time, self.world.day_time)
    }

    fn register_goals(&self, goals: &mut mod_api::GoalRegistrar<'_>) {
        self.native_register_goals(goals);
    }

    fn register_behaviors(&self, behaviors: &mut mod_api::BehaviorRegistrar<'_>) {
        self.native_register_behaviors(behaviors);
    }

    fn run_goal(
        &self,
        goal: &mut Box<dyn mod_api::Goal>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: GoalOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool> {
        let call = match op {
            GoalOp::CanUse => "goal_can_use",
            GoalOp::CanContinue => "goal_can_continue",
            GoalOp::Start => "goal_start",
            GoalOp::Stop => "goal_stop",
            GoalOp::Tick => "goal_tick",
        };
        let mut gcx = GoalCx { m, w, host: self };
        let mut context = mod_api::GoalContext::new(&mut gcx);
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: call,
            part: Some(part),
        });
        Some(match op {
            GoalOp::CanUse => goal.can_use(&mut context),
            GoalOp::CanContinue => goal.can_continue(&mut context),
            GoalOp::Start => {
                goal.start(&mut context);
                true
            }
            GoalOp::Stop => {
                goal.stop(&mut context);
                true
            }
            GoalOp::Tick => {
                goal.tick(&mut context);
                true
            }
        })
    }

    fn run_behavior(
        &self,
        behavior: &mut Box<dyn mod_api::Behavior>,
        m: &mut Mob,
        w: &MobWorld<'_>,
        op: BehaviorOp,
        mod_id: &'static str,
        part: &'static str,
    ) -> Option<bool> {
        let call = match op {
            BehaviorOp::CheckExtraStartConditions => "behavior_check_extra_start_conditions",
            BehaviorOp::Start => "behavior_start",
            BehaviorOp::CanStillUse => "behavior_can_still_use",
            BehaviorOp::Tick => "behavior_tick",
            BehaviorOp::Stop => "behavior_stop",
        };
        // The same view a goal runs on (#824): one boundary, the brain's
        // memories typed on top.
        let mut gcx = GoalCx { m, w, host: self };
        let mut context = mod_api::BehaviorCx::new(&mut gcx);
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: call,
            part: Some(part),
        });
        Some(match op {
            BehaviorOp::CheckExtraStartConditions => {
                behavior.check_extra_start_conditions(&mut context)
            }
            BehaviorOp::Start => {
                behavior.start(&mut context);
                true
            }
            BehaviorOp::CanStillUse => behavior.can_still_use(&mut context),
            BehaviorOp::Tick => {
                behavior.tick(&mut context);
                true
            }
            BehaviorOp::Stop => {
                behavior.stop(&mut context);
                true
            }
        })
    }
}

impl Game<'_> {
    /// Run `read` against the goal view of `mob`'s tick, as a mod goal reads.
    #[cfg(test)]
    pub(in crate::game) fn in_goal_view<R>(
        &self,
        mob: &mut Mob,
        read: impl FnOnce(&GoalCx<'_, '_>) -> R,
    ) -> R {
        let mut world = self.mob_world_base(&[]);
        world.goal_host = Some(self as &dyn mobs::part_goals::GoalHost);
        read(&GoalCx {
            m: mob,
            w: &world,
            host: self,
        })
    }
    /// Dispatch a target change against the mob tick's scoped read view. The
    /// game has lent its mob list to the tick, so using `Game` itself as the
    /// host here would make the actor and other mob handles unresolvable.
    pub(super) fn native_mob_change_target_in_tick(
        &self,
        mob: &mut Mob,
        world: &MobWorld<'_>,
        is_brain: bool,
        target: Option<Who>,
    ) -> mobs::TargetChange {
        let vanilla = mobs::TargetChange::Set(target);
        if !mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
            return vanilla;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return vanilla;
        };
        let original = match target {
            None => None,
            Some(Who::Mob(id)) => Some(LivingRef::new(EntityRef::new(epoch, id))),
            Some(Who::Player) => {
                let Some((_, id)) = world.selected_player_identity() else {
                    return vanilla;
                };
                Some(LivingRef::new(EntityRef::new(epoch, id)))
            }
        };
        let entity = MobRef::new(EntityRef::new(epoch, mob.id));
        let context = GoalCx {
            m: mob,
            w: world,
            host: self,
        };
        let mut new_target = original;
        let mut cancel = false;
        super::native_hooks::NativeHookHost { host: &context }.native_living_change_target(
            mod_api::__private::LivingChangeTargetCall {
                mob: entity,
                original_target: original,
                new_target: &mut new_target,
                is_brain,
                cancel: &mut cancel,
            },
        );
        if cancel {
            return mobs::TargetChange::Keep;
        }
        if new_target == original {
            return vanilla;
        }
        let Some(target) = new_target else {
            return mobs::TargetChange::Set(None);
        };
        if mobs::part_goals::GoalHost::is_fake_player(self, target.entity()) {
            return mobs::TargetChange::Keep;
        }
        // A listener can redirect to another real player in this level.
        // Select that player's stable tick view before the caller continues,
        // so `Who::Player` and subsequent AI reads name the same player.
        if target.entity().epoch() == epoch
            && let Some(selection) = world.player_selection
            && selection.get(target.entity().id()).is_some()
            && let Some(player) = self.native_entity_target(target.entity())
            && self.native_player_facts(player).is_some()
            && selection.select(target.entity().id())
        {
            return mobs::TargetChange::Set(Some(Who::Player));
        }
        if context.is_selected_player(target.entity()) {
            return mobs::TargetChange::Set(Some(Who::Player));
        }
        match context.resolve(target.entity()) {
            Some(Resolved::SyntheticPlayer) => mobs::TargetChange::Set(Some(Who::Player)),
            Some(Resolved::Own) => mobs::TargetChange::Set(Some(Who::Mob(context.m.id))),
            Some(Resolved::Live(mob)) => mobs::TargetChange::Set(Some(Who::Mob(mob.id))),
            Some(Resolved::Unavailable) | None => mobs::TargetChange::Keep,
        }
    }
}

pub(super) mod generated {
    // The mob tick's read view (#1077), as the game generates it: the
    // snapshot-backed reads forward to the context's own methods, every other
    // backend read answers its typed refusal.
    include!(concat!(env!("OUT_DIR"), "/mod_api_goal_view_generated.rs"));
}

#[cfg(test)]
#[path = "native_goal_ticks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_behavior_tests.rs"]
mod behavior_tests;
