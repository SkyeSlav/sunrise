//! Border reads carried from upstream through a level handle.
pub trait Notes {
    #[notes = r#"`native_world_border(dimension)`, the bounds of a level's world border as the server enforces it (one border for every level in Sunrise today)."#]
    fn world_border();
}
