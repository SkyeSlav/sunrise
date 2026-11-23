//! Entity.move ordering around collision, support, landing and movement effects.
use crate::{movement_emission::MovementEmission, registry::BlockStateId};
use std::collections::VecDeque;
use sunrise_core::{geometry::Vec3, position::BlockPos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoverType {
    SelfMovement,
    Player,
    Piston,
    ShulkerBox,
    Shulker,
    /// A moving collider carrying or shoving a body (Sunrise's own).
    Carried,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CollisionFlags {
    pub horizontal: bool,
    pub vertical: bool,
    pub below: bool,
    pub minor: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct MovementRecord {
    pub from: Vec3,
    pub to: Vec3,
    pub original: Option<Vec3>,
}
#[derive(Clone, Debug, Default)]
pub struct MovementHistory {
    records: VecDeque<MovementRecord>,
}
impl MovementHistory {
    pub fn records(&self) -> &VecDeque<MovementRecord> {
        &self.records
    }
    pub fn add(&mut self, record: MovementRecord) {
        if self.records.len() >= 100 {
            let first = self.records.pop_front().unwrap();
            let second = self.records.pop_front().unwrap();
            self.records.push_front(MovementRecord {
                from: first.from,
                to: second.to,
                original: None,
            });
        }
        self.records.push_back(record);
    }
    pub fn clear(&mut self) {
        self.records.clear();
    }
}
/// The provider retains entity fields so callbacks can mutate them. Hooks below
/// are native phase boundaries; bind collision, support, landing, restitution
/// and emission hooks to their corresponding world modules.
pub trait EntityMoveWorld {
    type Error;
    fn no_physics(&self) -> bool;
    fn position(&self) -> Vec3;
    fn set_position(&mut self, position: Vec3) -> Result<(), Self::Error>;
    fn record_movement(&mut self, mover: MoverType, movement: Vec3) -> Result<(), Self::Error>;
    fn collision_flags(&self) -> CollisionFlags;
    fn set_collision_flags(&mut self, flags: CollisionFlags);
    fn limit_piston_movement(&mut self, movement: Vec3) -> Result<Vec3, Self::Error>;
    fn stuck_multiplier(&self) -> Vec3;
    fn set_stuck_multiplier(&mut self, value: Vec3);
    fn velocity(&mut self) -> Result<Vec3, Self::Error>;
    fn set_velocity(&mut self, value: Vec3) -> Result<(), Self::Error>;
    fn back_off_edge(&mut self, movement: Vec3, mover: MoverType) -> Result<Vec3, Self::Error>;
    fn collide(&mut self, movement: Vec3) -> Result<Vec3, Self::Error>;
    fn fall_distance(&self) -> f64;
    /// FALLDAMAGE_RESETTING blocks and WATER fluids, entity collision context.
    /// Return true for a non-MISS hit.
    fn fall_reset_clip(&mut self, from: Vec3, to: Vec3) -> Result<bool, Self::Error>;
    fn reset_fall_distance(&mut self) -> Result<(), Self::Error>;
    fn movement_history(&mut self) -> &mut MovementHistory;
    fn locally_authoritative(&mut self) -> Result<bool, Self::Error>;
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        movement: Vec3,
    ) -> Result<(), Self::Error>;
    fn minor_horizontal_collision(&mut self, movement: Vec3) -> Result<bool, Self::Error>;
    fn legacy_on_position(&mut self) -> Result<BlockPos, Self::Error>;
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId, Self::Error>;
    fn on_ground(&mut self) -> Result<bool, Self::Error>;
    fn check_fall_damage(
        &mut self,
        dy: f64,
        on_ground: bool,
        state: BlockStateId,
        p: BlockPos,
    ) -> Result<(), Self::Error>;
    fn removed(&mut self) -> Result<bool, Self::Error>;
    fn can_simulate(&mut self) -> Result<bool, Self::Error>;
    fn restitute(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        collided_x: bool,
        collided_z: bool,
        resolved: Vec3,
    ) -> Result<(), Self::Error>;
    fn client_side(&mut self) -> Result<bool, Self::Error>;
    fn movement_emission(&mut self) -> Result<MovementEmission, Self::Error>;
    fn passenger(&mut self) -> Result<bool, Self::Error>;
    fn apply_movement_emission(
        &mut self,
        emission: MovementEmission,
        resolved: Vec3,
        p: BlockPos,
        state: BlockStateId,
    ) -> Result<(), Self::Error>;
    fn block_speed_factor(&mut self) -> Result<f32, Self::Error>;
}
fn java_zero(v: Vec3) -> bool {
    v.x.to_bits() == 0 && v.y.to_bits() == 0 && v.z.to_bits() == 0
}
/// `!Mth.equal(a, b)`: further apart than `1.0E-5F`, how `Entity.move` tells a blocked
/// sideways axis.
pub fn different(a: f64, b: f64) -> bool {
    let delta = (b - a).abs();
    delta >= f64::from(1.0e-5f32) || delta.is_nan()
}
/// Release behavior; native IDE-only invalid-simulation assertions and profiler
/// callbacks do not form part of this gameplay transition.
pub fn move_entity<W: EntityMoveWorld>(
    world: &mut W,
    mover: MoverType,
    movement: Vec3,
) -> Result<(), W::Error> {
    move_entity_until_landing(
        world,
        mover,
        movement,
        |_, _, _| Ok(None::<std::convert::Infallible>),
        |world, dy, grounded, state, p| {
            world.check_fall_damage(dy, grounded, state, p)?;
            Ok(None)
        },
    )
    .map(|_| ())
}

/// The part of `Entity.move` after its authoritative `checkFallDamage` call.
/// Keep the resolved movement and support block, not a borrow of the entity or level.
pub struct MoveTail {
    resolved: Vec3,
    x: bool,
    z: bool,
    nonzero_y: bool,
    p: BlockPos,
    state: BlockStateId,
}

/// Where a paused move resumes: `Section` after the position write (`Entity.setPosRaw`'s
/// section callback), `Landing` after `checkFallDamage`.
pub enum MoveStage {
    Section(MoveAfterWrite),
    Landing(MoveTail),
}

/// The locals `Entity.move` keeps across its position write: the move's own cause, what was
/// asked and what collision resolved. Everything after the write reads the entity again.
pub struct MoveAfterWrite {
    mover: MoverType,
    movement: Vec3,
    resolved: Vec3,
    /// The move was a no-physics one: it ends after recording the movement.
    no_physics: bool,
}

impl MoveStage {
    /// Continue the move: a `Section` stage runs on to the landing (which may pause again), a
    /// `Landing` stage finishes.
    pub fn resume<W: EntityMoveWorld, P>(
        self,
        world: &mut W,
        check_fall: impl FnOnce(
            &mut W,
            f64,
            bool,
            BlockStateId,
            BlockPos,
        ) -> Result<Option<P>, W::Error>,
    ) -> Result<Option<(P, MoveStage)>, W::Error> {
        match self {
            Self::Section(write) => write.resume(world, check_fall),
            Self::Landing(tail) => {
                tail.finish(world)?;
                Ok(None)
            }
        }
    }
}

/// Run a move, pausing after the position write if `check_section` asks (`from` is where the
/// entity was, `to` where it is) and before the tail if the landing needs the owning level.
pub fn move_entity_until_landing<W: EntityMoveWorld, P>(
    world: &mut W,
    mover: MoverType,
    mut movement: Vec3,
    check_section: impl FnOnce(&mut W, Vec3, Vec3) -> Result<Option<P>, W::Error>,
    check_fall: impl FnOnce(&mut W, f64, bool, BlockStateId, BlockPos) -> Result<Option<P>, W::Error>,
) -> Result<Option<(P, MoveStage)>, W::Error> {
    if world.no_physics() {
        let from = world.position();
        world.set_position(from.add_vector(movement))?;
        let to = world.position();
        let pause = check_section(world, from, to)?;
        let after = MoveAfterWrite {
            mover,
            movement,
            resolved: movement,
            no_physics: true,
        };
        if let Some(pause) = pause {
            return Ok(Some((pause, MoveStage::Section(after))));
        }
        return after.resume(world, check_fall);
    }
    if mover == MoverType::Piston {
        movement = world.limit_piston_movement(movement)?;
        if java_zero(movement) {
            return Ok(None);
        }
    }
    let stuck = world.stuck_multiplier();
    if stuck.length_squared() > 1.0e-7 {
        if mover != MoverType::Piston {
            movement = movement.multiply(stuck);
        }
        world.set_stuck_multiplier(Vec3::ZERO);
        world.set_velocity(Vec3::ZERO)?;
    }
    movement = world.back_off_edge(movement, mover)?;
    let resolved = world.collide(movement)?;
    let length = resolved.length_squared();
    if length > 1.0e-7 || movement.length_squared() - length < 1.0e-7 {
        if world.fall_distance() != 0.0 && length >= 1.0 {
            let distance = resolved.length().min(8.0);
            let from = world.position();
            let to = from.add_vector(resolved.normalize().scale(distance));
            if world.fall_reset_clip(from, to)? {
                world.reset_fall_distance()?;
            }
        }
        let from = world.position();
        let to = from.add_vector(resolved);
        world.movement_history().add(MovementRecord {
            from,
            to,
            original: Some(movement),
        });
        world.set_position(to)?;
        let after = MoveAfterWrite {
            mover,
            movement,
            resolved,
            no_physics: false,
        };
        if let Some(pause) = check_section(world, from, to)? {
            return Ok(Some((pause, MoveStage::Section(after))));
        }
        return after.resume(world, check_fall);
    }
    MoveAfterWrite {
        mover,
        movement,
        resolved,
        no_physics: false,
    }
    .finish_without_write(world, check_fall)
}

impl MoveAfterWrite {
    fn resume<W: EntityMoveWorld, P>(
        self,
        world: &mut W,
        check_fall: impl FnOnce(
            &mut W,
            f64,
            bool,
            BlockStateId,
            BlockPos,
        ) -> Result<Option<P>, W::Error>,
    ) -> Result<Option<(P, MoveStage)>, W::Error> {
        if self.no_physics {
            world.record_movement(self.mover, self.movement)?;
            world.set_collision_flags(CollisionFlags::default());
            return Ok(None);
        }
        let length = self.resolved.length_squared();
        if length > 1.0e-7 || self.movement.length_squared() - length < 1.0e-7 {
            world.record_movement(self.mover, self.resolved)?;
        }
        self.finish_without_write(world, check_fall)
    }

    fn finish_without_write<W: EntityMoveWorld, P>(
        self,
        world: &mut W,
        check_fall: impl FnOnce(
            &mut W,
            f64,
            bool,
            BlockStateId,
            BlockPos,
        ) -> Result<Option<P>, W::Error>,
    ) -> Result<Option<(P, MoveStage)>, W::Error> {
        let Self {
            movement, resolved, ..
        } = self;
        let x = different(movement.x, resolved.x);
        let z = different(movement.z, resolved.z);
        let mut flags = world.collision_flags();
        flags.horizontal = x || z;
        world.set_collision_flags(flags);
        let nonzero_y = movement.y.abs() > 0.0;
        if nonzero_y || world.locally_authoritative()? {
            flags = world.collision_flags();
            flags.vertical = movement.y != resolved.y;
            flags.below = flags.vertical && movement.y < 0.0;
            world.set_collision_flags(flags);
            world.set_on_ground_with_movement(flags.below, flags.horizontal, resolved)?;
        }
        let minor = if world.collision_flags().horizontal {
            world.minor_horizontal_collision(resolved)?
        } else {
            false
        };
        flags = world.collision_flags();
        flags.minor = minor;
        world.set_collision_flags(flags);
        let p = world.legacy_on_position()?;
        let state = world.block_state(p)?;
        let tail = MoveTail {
            resolved,
            x,
            z,
            nonzero_y,
            p,
            state,
        };
        if world.locally_authoritative()? {
            let grounded = world.on_ground()?;
            if let Some(pause) = check_fall(world, resolved.y, grounded, state, p)? {
                return Ok(Some((pause, MoveStage::Landing(tail))));
            }
        }
        tail.finish(world)?;
        Ok(None)
    }
}

impl MoveTail {
    pub fn finish<W: EntityMoveWorld>(self, world: &mut W) -> Result<(), W::Error> {
        let Self {
            resolved,
            x,
            z,
            nonzero_y,
            p,
            state,
        } = self;
        if world.removed()? {
            return Ok(());
        }
        if world.can_simulate()?
            && (nonzero_y && world.collision_flags().vertical || world.collision_flags().horizontal)
        {
            world.restitute(state, p, x, z, resolved)?;
        }
        if !world.client_side()? || world.locally_authoritative()? {
            let emission = world.movement_emission()?;
            if emission.anything() && !world.passenger()? {
                world.apply_movement_emission(emission, resolved, p, state)?;
            }
        }
        let factor = world.block_speed_factor()?;
        let velocity = scale_by_block_speed(world.velocity()?, factor);
        world.set_velocity(velocity)
    }
}
/// The last step of `Entity.move`: the horizontal velocity is multiplied by
/// `getBlockSpeedFactor()` (soul sand, honey), the vertical one is untouched.
/// Every kind of entity ends its move here, so the player and the mobs share
/// this one definition.
pub fn scale_by_block_speed(velocity: Vec3, factor: f32) -> Vec3 {
    let factor = f64::from(factor);
    velocity.multiply(Vec3::new(factor, 1.0, factor))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body in open air that logs each step of `Entity.move`.
    #[derive(Default)]
    struct Probe {
        position: Vec3,
        flags: CollisionFlags,
        history: MovementHistory,
        log: Vec<&'static str>,
    }

    impl EntityMoveWorld for Probe {
        type Error = ();
        fn no_physics(&self) -> bool {
            false
        }
        fn position(&self) -> Vec3 {
            self.position
        }
        fn set_position(&mut self, position: Vec3) -> Result<(), ()> {
            self.log.push("write");
            self.position = position;
            Ok(())
        }
        fn record_movement(&mut self, _: MoverType, _: Vec3) -> Result<(), ()> {
            self.log.push("record");
            Ok(())
        }
        fn collision_flags(&self) -> CollisionFlags {
            self.flags
        }
        fn set_collision_flags(&mut self, flags: CollisionFlags) {
            self.flags = flags;
        }
        fn limit_piston_movement(&mut self, movement: Vec3) -> Result<Vec3, ()> {
            Ok(movement)
        }
        fn stuck_multiplier(&self) -> Vec3 {
            Vec3::ZERO
        }
        fn set_stuck_multiplier(&mut self, _: Vec3) {}
        fn velocity(&mut self) -> Result<Vec3, ()> {
            Ok(Vec3::ZERO)
        }
        fn set_velocity(&mut self, _: Vec3) -> Result<(), ()> {
            self.log.push("velocity");
            Ok(())
        }
        fn back_off_edge(&mut self, movement: Vec3, _: MoverType) -> Result<Vec3, ()> {
            Ok(movement)
        }
        fn collide(&mut self, movement: Vec3) -> Result<Vec3, ()> {
            Ok(movement)
        }
        fn fall_distance(&self) -> f64 {
            0.0
        }
        fn fall_reset_clip(&mut self, _: Vec3, _: Vec3) -> Result<bool, ()> {
            Ok(false)
        }
        fn reset_fall_distance(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn movement_history(&mut self) -> &mut MovementHistory {
            &mut self.history
        }
        fn locally_authoritative(&mut self) -> Result<bool, ()> {
            Ok(true)
        }
        fn set_on_ground_with_movement(&mut self, _: bool, _: bool, _: Vec3) -> Result<(), ()> {
            self.log.push("ground");
            Ok(())
        }
        fn minor_horizontal_collision(&mut self, _: Vec3) -> Result<bool, ()> {
            Ok(false)
        }
        fn legacy_on_position(&mut self) -> Result<BlockPos, ()> {
            Ok(BlockPos::new(0, 0, 0))
        }
        fn block_state(&mut self, _: BlockPos) -> Result<BlockStateId, ()> {
            Ok(BlockStateId(0))
        }
        fn on_ground(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn check_fall_damage(
            &mut self,
            _: f64,
            _: bool,
            _: BlockStateId,
            _: BlockPos,
        ) -> Result<(), ()> {
            self.log.push("fall");
            Ok(())
        }
        fn removed(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn can_simulate(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn restitute(
            &mut self,
            _: BlockStateId,
            _: BlockPos,
            _: bool,
            _: bool,
            _: Vec3,
        ) -> Result<(), ()> {
            Ok(())
        }
        fn client_side(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn movement_emission(&mut self) -> Result<MovementEmission, ()> {
            Ok(MovementEmission::None)
        }
        fn passenger(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn apply_movement_emission(
            &mut self,
            _: MovementEmission,
            _: Vec3,
            _: BlockPos,
            _: BlockStateId,
        ) -> Result<(), ()> {
            Ok(())
        }
        fn block_speed_factor(&mut self) -> Result<f32, ()> {
            Ok(1.0)
        }
    }

    #[test]
    fn a_move_paused_after_its_write_resumes_with_the_rest_of_the_move() {
        let mut world = Probe::default();
        let paused = move_entity_until_landing(
            &mut world,
            MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
            |world, from, to| {
                assert_eq!((from.x, to.x), (0.0, 1.0));
                assert_eq!(world.log, ["write"], "nothing after the write has run yet");
                Ok(Some("section"))
            },
            |world, _, _, _, _| {
                world
                    .check_fall_damage(0.0, false, BlockStateId(0), BlockPos::new(0, 0, 0))
                    .map(|()| None)
            },
        )
        .unwrap();
        let (pause, stage) = paused.expect("the section callback paused the move");
        assert_eq!(pause, "section");
        // A listener's write while paused is seen by the rest of the move.
        world.position = Vec3::new(5.0, 0.0, 0.0);
        let again = stage
            .resume(&mut world, |world, _, _, _, _| {
                world.log.push("landing");
                Ok(None::<&str>)
            })
            .unwrap();
        assert!(again.is_none());
        assert_eq!(
            world.log,
            ["write", "record", "ground", "landing", "velocity"]
        );
        assert_eq!(world.position.x, 5.0);
    }
}
