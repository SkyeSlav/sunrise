use crate::game::native_surface::FrictionView;
use mod_api::{BlockSoundCause, BlockSoundType};
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl FrictionView<'_> {
    /// `IBlockExtension.getSoundType(state, level, pos, entity)`: a subscribed state is asked
    /// with the side's borrowed level, the position, the entity (if any) and the sound about to
    /// play.
    pub(in crate::game) fn native_sound_type(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
        sound: &mut BlockSoundType,
    ) {
        self.fire_sound_type(pos, state, entity, cause, sound);
    }
}
