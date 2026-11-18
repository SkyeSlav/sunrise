//! The collision part of `ServerGamePacketListenerImpl.handlePlayerPositionChange`, through the
//! guest's collision exclusions (#3717): a mod's boxes in which the level's blocks don't stop the
//! player are the same boxes its client moves it through.
use super::{Guest, Hub, TickCtx};
use crate::{
    collisions::{EntityCollisions, SceneCollisions},
    world::World,
};
use std::io;
use sunrise_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld},
    collision_context::{CollisionContext, CollisionEntity, CollisionItem, EntityContextKind},
    collision_exclusions::Excluding,
    entity_obstruction::EntityIdentity,
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
};

impl Hub {
    /// Where the guest ends moving from `from` towards `to` through the level as it stands now:
    /// where a carried player is committed. Nowhere to collide (no collision data) it is `to`.
    pub(super) fn settle(&self, i: usize, from: [f64; 3], to: [f64; 3], ctx: &TickCtx) -> [f64; 3] {
        let Some(binding) = ctx.collision else {
            return to;
        };
        self.settle_in(i, from, to, ctx.world, binding, &[])
            .unwrap_or(to)
    }

    /// [`Self::settle_in`] for the guest of connection `conn`; `None` for one the hub has no more.
    /// `entities` are the boxes of the solid entities it may meet.
    pub fn settle_conn(
        &self,
        conn: u32,
        from: [f64; 3],
        to: [f64; 3],
        world: &World,
        binding: &BlockCollisionRegistry<'_>,
        entities: &[Aabb],
    ) -> Option<[f64; 3]> {
        self.settle_in(
            self.guests.connection_index(conn)?,
            from,
            to,
            world,
            binding,
            entities,
        )
    }

    /// [`Self::settle`] through `world` and its collision data, `None` when the level cannot
    /// answer (a move too long for the collision query's budget): a caller that must not pass
    /// a wall on failure moves nothing.
    fn settle_in(
        &self,
        i: usize,
        from: [f64; 3],
        to: [f64; 3],
        world: &World,
        binding: &BlockCollisionRegistry<'_>,
        entities: &[Aabb],
    ) -> Option<[f64; 3]> {
        let g = &self.guests[i];
        let movement = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        let d = g.dimensions();
        let half = f64::from(d.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(from[0] - half, from[1], from[2] - half),
            Vec3::new(
                from[0] + half,
                from[1] + f64::from(d.height),
                from[2] + half,
            ),
        );
        let context = guest_context(g, from[1], g.sneaking);
        let mut scene = EntityCollisions {
            blocks: SceneCollisions {
                world,
                binding,
                subject: None,
            },
            entities,
            border: None,
        };
        let done = if g.collision_exclusions.is_empty() {
            settled(g, bounds, movement, context, &mut scene)
        } else {
            settled(
                g,
                bounds,
                movement,
                context,
                &mut Excluding {
                    world: &mut scene,
                    boxes: g.collision_exclusions.boxes(),
                },
            )
        };
        done.map(|done| [from[0] + done.x, from[1] + done.y, from[2] + done.z])
    }

    /// The collision part of handlePlayerPositionChange: whether the move
    /// counts as moved wrongly (logged), and whether it is refused (the
    /// player is sent back where they were).
    pub(super) fn collide(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> (bool, bool, bool) {
        self.collide_from(i, self.guests[i].position, to, ctx)
    }

    /// [`Self::collide`] for a move that starts at `from` instead of where the guest stands:
    /// where a carried player's last position is under the collider's transform of the report.
    pub(super) fn collide_from(
        &self,
        i: usize,
        from: [f64; 3],
        to: [f64; 3],
        ctx: &TickCtx,
    ) -> (bool, bool, bool) {
        let g = &self.guests[i];
        // noPhysics (spectators) and sleepers move freely.
        let Some(binding) = ctx.collision else {
            return (false, false, false);
        };
        if g.spectator() || g.sleeping {
            return (false, false, false);
        }
        let mut scene = SceneCollisions {
            world: ctx.world,
            binding,
            subject: None,
        };
        // A Player-cause move through the move gate (#2414); the gate records the movement
        // unchanged and its delta is what the collision resolves.
        let requested = Vec3::new(to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        let gate = crate::move_gate::armed(
            &self.loop_context,
            g.entity_id as u32,
            "minecraft:player",
            sunrise_world::entity_move::MoverType::Player,
            requested,
        );
        let movement = gate.delta();
        // A listener's decision that is not the request (#2414, ruling 5): the
        // flag comes from the gate, not from comparing deltas here — a walk's
        // step is below «moved wrongly»'s threshold, and its cancel must
        // correct the guest all the same.
        let overridden = gate.overridden();
        // The guest's move, as the entity a block's parts are asked about (the gate names it
        // for its own moves): the server asks the same question the guest's client did.
        sunrise_client_render::moving_colliders::as_mover(g.entity_id as u32, || {
            // A move with no exclusions pays this one check.
            if g.collision_exclusions.is_empty() {
                return check_move(g, from, to, requested, movement, overridden, &mut scene);
            }
            let boxes = g.collision_exclusions.boxes();
            check_move(
                g,
                from,
                to,
                requested,
                movement,
                overridden,
                &mut Excluding {
                    world: &mut scene,
                    boxes,
                },
            )
        })
    }
}

/// The context the guest's own client moves through (`MotionWorld::context`): a player at `y`
/// holding the item in its selected slot, so a part is asked the same query on both sides. The
/// identity is the guest's own, whoever the move's collider exemption names (a carried guest's
/// carrier, say), so a part is asked about the guest in every scan of its moves.
fn guest_context(g: &Guest, y: f64, descending: bool) -> CollisionContext<'_> {
    let held = g
        .inventory
        .get(g.selected)
        .and_then(Option::as_ref)
        .filter(|stack| stack.count > 0)
        .map_or(CollisionItem::EMPTY, |stack| CollisionItem {
            item: &stack.item,
            count: 1,
        });
    CollisionContext::for_entity(
        CollisionEntity {
            identity: EntityIdentity(u64::from(g.entity_id as u32)),
            kind: EntityContextKind::Player,
            y,
            descending,
            main_hand: Some(held),
        },
        false,
    )
}

/// How far the guest's box `bounds` gets by `movement` in `world`, as `Entity.move` resolves it.
fn settled<W>(
    g: &Guest,
    bounds: Aabb,
    movement: Vec3,
    context: CollisionContext,
    world: &mut W,
) -> Option<Vec3>
where
    W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
{
    resolve_world_movement(
        world,
        MovementRequest {
            entity: EntityIdentity(0),
            context,
            bounds,
            movement,
            max_step: 0.6,
            on_ground: g.on_ground,
            candidate_budget: 1_000_000,
        },
    )
    .ok()
}

/// [`Hub::collide`] against `world`'s blocks.
fn check_move<W>(
    g: &Guest,
    from: [f64; 3],
    to: [f64; 3],
    requested: Vec3,
    movement: Vec3,
    overridden: bool,
    world: &mut W,
) -> (bool, bool, bool)
where
    W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
{
    // getBoundingBox: the box of the body's pose (swimming and crawling
    // are 0.6 tall), so a swimmer under a ceiling isn't sent back.
    let d = g.dimensions();
    let (half, height) = (f64::from(d.width) / 2.0, f64::from(d.height));
    let at = |p: [f64; 3]| {
        Aabb::new(
            Vec3::new(p[0] - half, p[1], p[2] - half),
            Vec3::new(p[0] + half, p[1] + height, p[2] + half),
        )
    };
    let (old, new) = (at(from), at(to));
    // player.move, then how far from the target it ended (vanilla drops
    // the vertical part: `yDist > -0.5 || yDist < 0.5` always holds).
    // The mover's own sneaking, as CollisionContext.of(player): a sneaking
    // player descends through scaffolding.
    let context = guest_context(g, from[1], g.sneaking);
    let done = settled(g, old, movement, context, world);
    // The gate decides what actually moves: the requested delta, a listener's
    // replacement, or nothing (a cancel). «Moved wrongly» still measures the
    // **requested** move against what happened, as vanilla measures the
    // packet's target - a cancelled move that went nowhere is the full
    // discrepancy. A decision that is not the request fails whatever the
    // distance (#2414, ruling 5): a cancel or a replacement always sends the
    // guest a correction, so a walk's small step cannot slip under the
    // threshold and keep going. Vanilla's exemptions - a creative guest, and
    // the unobstructed-start check below - belong to the distance test alone
    // (review round 1): an overridden move is refused for a creative guest
    // and from an obstructed start too.
    let moved_wrongly = !g.creative()
        && done.is_some_and(|done| {
            let (dx, dz) = (requested.x - done.x, requested.z - done.z);
            dx * dx + dz * dz > 0.0625
        });
    let fail = overridden || moved_wrongly;
    // Entity.move: verticalCollisionBelow, the player "stands on something".
    let stands = done.is_some_and(|done| movement.y < 0.0 && done.y != movement.y);
    if fail {
        let unobstructed = no_world_collision(
            world,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context: guest_context(g, old.min_position().y, false),
                bounds: old,
                candidate_budget: 1_000_000,
            },
        );
        // An overridden decision is refused from wherever it starts: the
        // unobstructed-old-box exemption is the distance test's, like
        // creative above (review round 1).
        if overridden || unobstructed.unwrap_or(true) {
            return (true, true, stands);
        }
    }
    // isEntityCollidingWithAnythingNew: a block the old box did not
    // already touch.
    let shrink = f64::from(1.0E-5_f32);
    let shrink = Vec3::new(shrink, shrink, shrink);
    // The same for each box of a moving collider (a mod's `set_collider`): one the old box was
    // not already in, box by box. Outside a level that holds colliders this answers at once.
    if sunrise_client_render::moving_colliders::new_collision(
        EntityIdentity(g.entity_id as u64),
        old.deflate(shrink),
        new.deflate(shrink),
    ) {
        return (fail, true, stands);
    }
    let Ok(old_shape) = VoxelShape::from_box(old.deflate(shrink)) else {
        return (fail, false, stands);
    };
    let Ok(scan) = BlockCollisionScan::new(world, context, new.deflate(shrink), false, 1 << 20)
    else {
        return (fail, false, stands);
    };
    for hit in scan {
        let Ok(hit) = hit else { break };
        if !hit
            .shape
            .join_is_not_empty(&old_shape, ShapeOp::AND)
            .unwrap_or(true)
        {
            return (fail, true, stands);
        }
    }
    (fail, false, stands)
}
