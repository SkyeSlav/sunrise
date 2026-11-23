//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The host's methods both sides share of the `resources` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ResourcesHost {
    /// Side: both.
    /// The winning bytes of resource `id` in `pack_type`'s stack, as
    /// `ResourceManager.getResource` answers it: the highest selected pack that
    /// supplies the file wins. Client resources are the selected asset packs over
    /// the game's assets; server data is the world's selected data packs over the
    /// game's and mods' data, with `minecraft:vanilla` kept unless disabled.
    /// `identifier` is the resource name without its `assets/<namespace>/` or
    /// `data/<namespace>/` prefix, e.g. `minecraft:models/block/stone.json` or
    /// `mymod:machines/furnace.json`. Missing, unreadable and files above
    /// [`MAX_RESOURCE_BYTES`](crate::MAX_RESOURCE_BYTES) return `None`; a client side (or one without that
    /// stack) also returns `None`. This read never reloads packs.
    fn resource(&self, pack_type: PackType, identifier: &Identifier) -> Option<Vec<u8>>;

    /// Side: both.
    /// Every selected pack's copy of resource `identifier` in `pack_type`'s stack, lowest first
    /// and the highest pack's last, as `ResourceManager.getResourceStack` answers it; empty where
    /// no pack supplies it. The stack and the names are those of [`resource`](Self::resource),
    /// which reads the highest pack's copy. A copy that can't be read or is above
    /// [`MAX_RESOURCE_BYTES`](crate::MAX_RESOURCE_BYTES) is left out: when that is the highest
    /// pack's copy, the last entry is a lower pack's while `resource` returns `None`. This read
    /// never reloads packs.
    fn resource_stack(&self, pack_type: PackType, identifier: &Identifier) -> Vec<Vec<u8>>;

    /// Side: both.
    /// The resources of `pack_type`'s stack under folder `directory` of every namespace whose
    /// path ends in `suffix`, one each, in identifier order, as `ResourceManager.listResources`
    /// answers it: `list_resources(PackType::ClientResources, "textures/item", ".png")` gives
    /// `minecraft:textures/item/apple.png` and the rest. An empty `directory` lists a whole
    /// namespace. Metadata files (`.mcmeta`) are never listed, and a name that isn't a valid
    /// identifier is skipped. A side without that stack, or a `directory` that leaves its
    /// namespace, lists nothing. This read never reloads packs.
    fn list_resources(&self, pack_type: PackType, directory: &str, suffix: &str)
    -> Vec<Identifier>;
}
