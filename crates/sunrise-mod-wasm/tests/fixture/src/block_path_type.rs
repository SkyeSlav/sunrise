//! The same listener source runs natively, in wire loopback and in the compiled guest.
use mod_api::{BlockPart, BlockPathTypeEvent, LevelHost, PathType, Tag};

pub struct PathPart;

#[mod_api::sunrise_part]
impl BlockPart for PathPart {
    fn block_path_type(
        &self,
        level: &dyn LevelHost,
        event: &mut BlockPathTypeEvent,
    ) -> Option<PathType> {
        assert_eq!(event.pos, mod_api::BlockPos::new(1, 64, 1));
        // A neighbouring block entity is read through the owning level, then
        // its saved data, on the same call path the server uses.
        let entity = level
            .block_entity([0, 64, 1])
            .expect("the chest beside the queried block");
        let data = level
            .block_entity_data(entity)
            .expect("the chest's saved data");
        assert_eq!(data.get(&"count".into()), Some(&Tag::Int(3)));
        Some(PathType::Damaging)
    }
}

/// Registered on `minecraft:stone` when the compiled-guest test asks for this part
/// (`block_path_type_probe`), so the fixture's other guest tests are unaffected.
pub fn register(r: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {
    if cx.mods().get("block_path_type_probe").is_none() {
        return;
    }
    let stone = r.block_key("minecraft:stone");
    r.block_part(stone, PathPart);
}
