//! The `player_menus` area: the menu a player has open. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_menus` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerMenusBackend {
    /// Side: both.
    /// A player's current supported menu. Refuses crafting tables, merchants,
    /// workstations, lecterns, mounts and menus with hidden slots; also refuses
    /// clients, stale players and players whose state is swapped out.
    #[access(WorldOnly)]
    fn player_container_menu(&self, player: PlayerRef) -> Option<MenuRef>;
}
/// The handles' writes of the `player_menus` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerMenusBackendMut {
    /// Side: server.
    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    #[access(WorldOnly)]
    fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError>;
    /// Side: server.
    /// Opens content mod menu type `menu` for a connected player from anywhere, with no
    /// block (`ServerPlayer.openMenu(provider, extraDataWriter)`): an item's use, an entity,
    /// a command. The menu replaces the one they had open. Its container slots show empty
    /// and their clicks go to the mods (`native_menu_clicked`), as `open_menu`'s do. `extra`
    /// (at most 32 KiB) is what the client's `screen_extra_data` reads, and `pos`, when
    /// given, its `screen_pos`. The old menu closes on the server as a close does (its
    /// `removed`, `player_container_close`, the cursor stack and crafting grids back to the
    /// inventory) before the new one is built; `replace` says whether the client is first sent
    /// a close for it, as for `open_menu`. `MenuReplace::InPlace` sends none: the client keeps
    /// its screen and mouse until the new screen replaces it. The window id, or Ok(None) when
    /// a mod cancelled the open. Errors: `NotModMenu`, `PlayerUnavailable` (stale, fake or
    /// vanilla client), `TooLarge`, or `Title` for a title that could not cross; a cancelled or
    /// refused open leaves the old menu as it was.
    #[access(WorldOnly)]
    fn player_open_mod_menu(
        &mut self,
        player: PlayerRef,
        menu: MenuKey,
        title: Text,
        extra: Vec<u8>,
        pos: Option<[i32; 3]>,
        replace: MenuReplace,
    ) -> Result<Option<i32>, OpenMenuError>;
    /// Side: server.
    /// Backs the container slots of the mod menu `window` that `player_open_mod_menu` opened
    /// for the player with storage that is not a block entity (#3385; NeoForge's
    /// `StackCopySlot` and `ResourceHandlerSlot`): `places[n]` is what slot `n` shows. From then
    /// on a click or shift-click on those slots, and an `ItemPlace::MenuSlot` write, go through
    /// that storage's own handler, journalled; the client sees ordinary synced slots, mirrored
    /// after each click and each write through an `ItemPlace` of that storage, never each tick.
    /// A slot past `places` stays the mods' own (`native_menu_clicked`). The menu closes once a
    /// place stops resolving (the backpack left the hand, the entity is gone) or a held stack is
    /// no longer the one bound when the slots were backed, checked before each click and at
    /// each change notice. A `MenuSlotPlace::HeldItem` locks the inventory slot holding that
    /// stack while the menu is open, so it can't go into itself. Backing again replaces the
    /// places; a slot no longer backed (an empty list, a shorter one) is emptied, never left
    /// holding the copy it showed. A [`MenuSlotBackRefusal`], and nothing changes, for a player
    /// not online or a window that is not their open menu, a menu that is not a blockless mod
    /// menu, more places than its container slots, or a place that doesn't resolve now.
    #[access(WorldOnly)]
    fn player_back_mod_menu_slots(
        &mut self,
        player: PlayerRef,
        window: i32,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal>;
}
