//! The `menu_still_valid` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a player's mod menu stays open (`AbstractContainerMenu.stillValid`).
    /// Asked on the server each tick the menu is open and before processing a
    /// matching non-spectator container click.
    /// `pos` is the backing block's position, or `None` for a blockless menu
    /// even if its opener supplied a position for the client's screen.
    /// `valid` starts true without a backing block; for a block menu it is
    /// whether `eye` is within the player's block interaction range plus 4 of
    /// each backing block. Set it to keep the menu open or close it. Vanilla
    /// menu types are never asked, and a menu whose block entity is gone
    /// closes without asking. Answer from the event and loop context alone.
    #[notes = r##"### Keeping a mod menu open

`menu_still_valid` is vanilla's `AbstractContainerMenu.stillValid(Player)`, the method a Java menu overrides to
say whether a player may keep it open. The server asks the scoped `MenuPart` of the open mod menu's type each
tick the menu is open and before processing a matching non-spectator container click. A click for a different
window is dropped; a matching spectator click resends the menu without asking the part. Vanilla menu types
keep their own check and never ask a part. A false answer closes the menu as walking away does: what the
cursor held goes back to the inventory and the client's window closes.

`event.player` is the player's UUID and `event.menu` is the menu type. `event.pos` is the backing block's
position, or `None` for a blockless menu even if its opener supplied a position for the client's screen.
`event.valid` starts true for blockless menus. For block menus it starts true only if the player's eyes
(`event.eye`) are within their block interaction range plus 4 of each backing block. A part may keep a larger
machine's menu open past that distance, or close a menu for any reason. A part that leaves `valid` alone, or
whose mod is switched off, keeps the game's answer. A menu whose block entity is gone closes without asking;
respawning, changing dimension or leaving also closes the menu.

The hook runs in the loop context (no level or game Host) on every open menu of the type each tick, so answer
from the event and the context alone, and fast. Whether the type has a part that answers is looked up once,
when the menu opens: a menu with none does no work for this hook."##]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        context = "loop",
        version = 2,
        non_exhaustive = true,
        target = "sunrise_client_lib::lan::menu::native_menu_still_valid",
        neoforge = "none (vanilla AbstractContainerMenu.stillValid, which each menu overrides)"
    )]
    fn menu_still_valid();
}
