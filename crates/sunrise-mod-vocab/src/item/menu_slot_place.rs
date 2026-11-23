//! Storage that is not a block entity behind a mod menu's slots (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` (a stack held inside another item) and `ResourceHandlerSlot`
//! (an index of an item handler). A mod opens its menu type with `PlayerRef::open_mod_menu`
//! (#3384) and then backs its container slots with [`PlayerRef::back_mod_menu_slots`]; a click
//! or shift-click on such a slot, and an [`ItemPlace::MenuSlot`] write to it, go through that
//! storage's own handler in one journalled transaction, as a block entity's slots do.
use crate::{
    Capability, ENTITY_ITEMS, EntityRef, Hand, ITEM_ITEMS, ItemPlace, ItemResource, PlayerRef,
    ResourceHandler, ServerHost, backend_of,
};

/// The storage one container slot of a mod menu opened with no block shows
/// ([`MenuSlotBacking::Place`](crate::MenuSlotBacking::Place)).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuSlotPlace {
    /// Slot `slot` of the container inside the stack the menu's player holds in `hand`, through
    /// that stack's [`ITEM_ITEMS`] handler (`StackCopySlot`). Writes update the actual selected
    /// hotbar or off-hand inventory slot in every game mode (`ItemAccess.forPlayerSlot`), not
    /// the hand interaction (`ItemAccess.forPlayerInteraction`). The hotbar slot holding a
    /// main-hand container takes no click while the menu is open, so it can't go into itself.
    /// The stack is bound when the slots are backed. Direct writes refuse a changed binding;
    /// the menu closes before the next click or at the next change notice if the bound stack
    /// was replaced or the selected hotbar slot holding it changed.
    HeldItem {
        /// The hand holding the container.
        hand: Hand,
        /// The slot's index in the container.
        slot: usize,
    },
    /// Slot `slot` of `entity`'s [`ENTITY_ITEMS`] handler, as a mod's provider answers it for that
    /// entity (`ResourceHandlerSlot`). The menu closes once the entity is gone, found before the
    /// next click or at the next change notice.
    EntityItems {
        /// The entity holding the inventory.
        entity: EntityRef,
        /// The slot's index in the handler.
        slot: usize,
    },
}

/// Why [`PlayerRef::back_mod_menu_slots`] backed nothing; the menu and its slots stay as they
/// were.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuSlotBackRefusal {
    /// The player is not online here (also a host with no game behind it), or `window` is not
    /// the menu they have open.
    NoPlayerOrWindow,
    /// The open menu cannot show another storage: it is not a blockless mod menu, or
    /// its container slots are declared as transient, menu-owned storage.
    UnsupportedMenu,
    /// More places than the menu has container slots.
    TooManyPlaces,
    /// `places[index]` names no storage now (no container in that hand, the entity gone or
    /// without an item handler, a slot past the handler's size).
    PlaceUnresolved {
        /// The place's index in `places`.
        index: usize,
    },
}

impl crate::NoBackend for MenuSlotBackRefusal {
    /// No game behind the host: no player is online there.
    fn no_backend() -> Self {
        Self::NoPlayerOrWindow
    }
}

impl MenuSlotPlace {
    /// The handler and its in-range index for the menu of `player`, if the place resolves.
    pub(crate) fn handler(
        self,
        host: &mut dyn ServerHost,
        player: PlayerRef,
    ) -> Option<(Capability<dyn ResourceHandler<ItemResource>>, usize)> {
        let (handler, slot) = match self {
            Self::HeldItem { hand, slot } => {
                let inventory_slot = match hand {
                    Hand::Main => {
                        i32::try_from(backend_of(host)?.player_selected_slot(player)?).ok()?
                    }
                    Hand::Off => -106, // ItemPlace::PlayerSlot's off-hand address.
                };
                let held = ItemPlace::PlayerSlot {
                    player: player.uuid(),
                    slot: inventory_slot,
                };
                (held.capability(host, &ITEM_ITEMS)?, slot)
            }
            Self::EntityItems { entity, slot } => {
                (entity.capability(host, &ENTITY_ITEMS, ())?, slot)
            }
        };
        (slot < handler.size(host)).then_some((handler, slot))
    }

    /// Whether this place still names storage, for the menu of `player`: false closes the menu.
    pub fn resolves(self, host: &mut dyn ServerHost, player: PlayerRef) -> bool {
        self.handler(host, player).is_some()
    }
}
