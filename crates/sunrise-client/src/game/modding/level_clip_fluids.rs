//! `ClipContext.Fluid` on a level ray (`BlockGetter.clip`): a fluid state counts by its mode
//! (`ClipContext.Fluid.canPick`) and is hit at its `FluidState.getShape`, through the same
//! process-wide first-query shape cache entities' fluid checks read (`FlowingFluid.shapes`).
use super::explosion::ColliderRays;
use std::io;
use sunrise_client::{items::Items, motion::FluidResources, world::World};
use sunrise_core::{
    geometry::{Direction, VoxelShape},
    position::BlockPos,
};
use sunrise_world::{
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    registry::BlockStateId,
    world_ray::{RayBlockMode, RayWorld},
};

/// A level ray that also reads fluids: blocks as [`ColliderRays`] reads them.
pub(super) struct FluidClip<'a, 'r> {
    pub(super) rays: ColliderRays<'a, 'r>,
    pub(super) fluids: &'a FluidResources,
    /// The effective fluid tags (`FluidTags.WATER`), read only for `ClipFluid::Water`.
    pub(super) tags: Option<&'a Items>,
}

/// The ray's world, as a fluid's height reads its neighbours: a column the world doesn't hold
/// is no answer, so no shape is cached from it; above or below the world is empty.
struct Column<'a> {
    world: &'a World,
    fluids: &'a FluidResources,
}
impl Column<'_> {
    fn state(&self, [x, y, z]: [i32; 3]) -> io::Result<BlockStateId> {
        let world = self.world;
        if (world.bottom_y()..world.top_y()).contains(&y)
            && !world.chunks.contains_key(&(x >> 4, z >> 4))
        {
            return Err(io::Error::other("fluid shape read an unloaded block"));
        }
        Ok(world.block(x, y, z))
    }
}
impl FluidHeightWorld for Column<'_> {
    type Error = io::Error;
    fn fluid(&mut self, pos: [i32; 3]) -> io::Result<FluidState> {
        Ok(self.fluids.fluid(self.state(pos)?))
    }
    fn sturdy_down(&mut self, pos: [i32; 3]) -> io::Result<bool> {
        Ok(self.fluids.sturdy(self.state(pos)?, Direction::Down))
    }
}

impl RayWorld for FluidClip<'_, '_> {
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = FluidState;
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        self.rays.block_state(p)
    }
    fn fluid_state(&mut self, p: BlockPos) -> io::Result<FluidState> {
        Ok(self.fluids.fluid(self.rays.block_state(p)?))
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        mode: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        self.rays.block_shape(state, p, mode)
    }
    fn interaction_shape(&mut self, state: BlockStateId, p: BlockPos) -> io::Result<VoxelShape> {
        self.rays.interaction_shape(state, p)
    }
    fn fluid_is_source(&mut self, state: FluidState) -> io::Result<bool> {
        Ok(state.is_source())
    }
    fn fluid_is_empty(&mut self, state: FluidState) -> io::Result<bool> {
        Ok(state.kind() == FluidKind::Empty)
    }
    fn fluid_is_water(&mut self, state: FluidState) -> io::Result<bool> {
        let tags = self
            .tags
            .ok_or_else(|| io::Error::other("no fluid tags to read minecraft:water from"))?;
        Ok(tags.fluid_in_tag("minecraft:water", state.kind().name()))
    }
    fn fluid_shape(&mut self, state: FluidState, p: BlockPos) -> io::Result<VoxelShape> {
        let mut column = Column {
            world: self.rays.scene.world,
            fluids: self.fluids,
        };
        let mut shapes = self
            .fluids
            .shapes
            .lock()
            .map_err(|_| io::Error::other("fluid shape cache poisoned"))?;
        shapes.shape(&mut column, state, p)
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
    use mod_api::{BlockFace, ClipBlock, ClipFluid};
    use std::sync::Arc;
    use sunrise_client::chunk::Chunk;

    /// `ClipContext.Fluid`'s modes on `LevelRef::clip`: flowing water of amount 4 is hit at its
    /// surface by `Any` and `Water` only, source lava by `SourceOnly` and `Any` only.
    #[test]
    #[ignore = "daily"]
    fn level_clip_stops_at_the_fluids_its_mode_picks() {
        if run_fresh_cases(
            "game::level_clip_fluids::tests::level_clip_stops_at_the_fluids_its_mode_picks",
            "SUNRISE_LEVEL_CLIP_FLUIDS_CASE",
            &["modes"],
        ) {
            return;
        }
        with_game(|game| {
            let dir = std::env::temp_dir()
                .join(format!("sunrise-level-clip-fluids-{}", std::process::id()));
            game.session_lock = Some(sunrise_client::save::SessionLock::acquire(&dir).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let water = registry.default_state("minecraft:water").unwrap();
            let water = game.with_props(water, &[("level", "4")]).unwrap();
            let lava = registry.default_state("minecraft:lava").unwrap();
            game.world.set(0, 64, 0, water).unwrap();
            game.world.set(2, 64, 0, lava).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let down = |game: &crate::game::Game<'_>, x: f64, fluids| {
                level
                    .clip(
                        game,
                        [x, 67.0, 0.5],
                        [x, 63.0, 0.5],
                        ClipBlock::Outline,
                        fluids,
                    )
                    .unwrap()
            };
            let surface = 64.0 + f64::from(4.0_f32 / 9.0);
            for fluids in [ClipFluid::Any, ClipFluid::Water] {
                let hit = down(game, 0.5, fluids);
                assert!(!hit.miss, "{fluids:?}");
                assert_eq!(
                    (hit.block, hit.face, hit.location),
                    ([0, 64, 0], BlockFace::Up, [0.5, surface, 0.5]),
                    "{fluids:?}"
                );
            }
            for fluids in [ClipFluid::None, ClipFluid::SourceOnly] {
                let miss = down(game, 0.5, fluids);
                assert!(miss.miss, "{fluids:?}");
                assert_eq!(miss.location, [0.5, 63.0, 0.5], "{fluids:?}");
            }
            for fluids in [ClipFluid::SourceOnly, ClipFluid::Any] {
                let hit = down(game, 2.5, fluids);
                assert!(!hit.miss, "{fluids:?}");
                assert_eq!(
                    (hit.block, hit.location),
                    ([2, 64, 0], [2.5, 64.0 + f64::from(8.0_f32 / 9.0), 0.5]),
                    "{fluids:?}"
                );
            }
            for fluids in [ClipFluid::None, ClipFluid::Water] {
                assert!(down(game, 2.5, fluids).miss, "{fluids:?}");
            }
        });
    }
}
