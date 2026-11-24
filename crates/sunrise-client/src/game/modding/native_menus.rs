//! Server menus for native mods (docs/modding/src/how-to/native-mods.md, "Server menus"): a vanilla chest menu (`generic_9x1` to `generic_9x6`) a mod opens
//! for a player and fills with items to look at. No click moves anything:
//! each reaches the mod (`native_menu_clicked`), as does the menu's closing
//! (`native_menu_closed`), and the client shows the menu as the server sends
//! it again. A stock client shows it; nothing on the player's side. A trade
//! built on these menus moves the items between the players' `ENTITY_ITEMS`
//! handlers in one transaction, so nothing moves before both sides are settled.
use super::{Game, lan::LanHost, lan_menus::GuestMenus};
use mod_api::__private::game::*;
use mod_api::{MenuRef, MenuReplace};
use sunrise_client::{
    items::ItemStack,
    lan::{
        data::component_nbt,
        menu::{Menu, NativeMenuEvent, Slots},
    },
    text::Text,
};

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(crate) fn native_menu_inventory_slot(size: usize, slot: i32) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// Adds every stack of `gets` to the 36 slots of `inventory`, onto matching
/// stacks first, then into empty slots, hotbar first; false when one doesn't
/// fit whole. `max` is an item's stack size.
pub(super) fn fit(
    inventory: &mut [Option<ItemStack>],
    gets: &[ItemStack],
    max: &impl Fn(&ItemStack) -> u32,
) -> bool {
    for stack in gets {
        let mut left = stack.count;
        let max = max(stack).max(1);
        for slot in inventory.iter_mut().take(36).flatten() {
            if left > 0 && slot.same_item_same_components(stack) && slot.count < max {
                let n = (max - slot.count).min(left);
                slot.count += n;
                left -= n;
            }
        }
        for slot in inventory.iter_mut().take(36) {
            if left > 0 && slot.is_none() {
                let n = max.min(left);
                *slot = Some(ItemStack {
                    count: n,
                    ..stack.clone()
                });
                left -= n;
            }
        }
        if left > 0 {
            return false;
        }
    }
    true
}

/// Server menus, for server mods. The players are named by UUID;
/// none of these runs while a player's state is swapped in (inside
/// `native_run_mod_command`, the menus can be opened and filled).
#[allow(dead_code)]
impl Game<'_> {
    pub(super) fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.participates_online() && g.uuid == uuid)?;
        Some(g.conn)
    }
    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its window id, a new one each time. The old
    /// menu closes as a close does, its cursor stack going back to the
    /// inventory, and `replace` says whether the client is first sent a close.
    pub(super) fn open_menu(
        &mut self,
        uuid: u128,
        rows: u8,
        title: Text,
        items: Vec<Option<ItemStack>>,
        replace: MenuReplace,
    ) -> Option<i32> {
        let conn = self.native_conn(uuid)?;
        self.close_for_open(conn, replace);
        let gi = self.guest_index(conn)?;
        // The first contents show the player's own inventory and cursor, in their turn too.
        let window = self.with_own_record(gi, |lan, _| {
            lan.hub
                .open_native_menu(conn, rows, component_nbt(&title.component()), items)
        })?;
        self.player_menu_opening(conn);
        Some(window)
    }
    /// `ServerPlayer.openMenu`'s close of the menu that was open: `closeContainer`, which then
    /// tells the client, or `doCloseContainer` for a provider that skips the client's close.
    /// Either way the old menu's `removed` runs and the mods hear it close.
    pub(super) fn close_for_open(&mut self, conn: u32, replace: MenuReplace) {
        match replace {
            MenuReplace::CloseFirst => self.guest_close_menus_and_tell(conn),
            MenuReplace::InPlace => self.guest_close_all(conn),
        }
    }
    /// `ServerPlayer.openMenu(provider, extraDataWriter)` for a content mod menu type and no
    /// block: the type's container slots show empty and their clicks reach the mods, as
    /// `open_menu`'s do; `extra` and `pos` go first in its `sunrise:menu_extra` payload, and
    /// `replace` is the provider's choice of the client's close, as for `open_menu`. The slots
    /// its screen hides at open (`native_hidden_slots`) take no shift-click there.
    pub(super) fn player_open_mod_menu(
        &mut self,
        player: mod_api::PlayerRef,
        menu: mod_api::MenuKey,
        title: Text,
        extra: Vec<u8>,
        pos: Option<[i32; 3]>,
        replace: MenuReplace,
    ) -> Result<Option<i32>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        let id = self.menu_key_id(menu);
        let kind =
            sunrise_client::containers::mod_menu_type(&id).ok_or(OpenMenuError::NotModMenu)?;
        if extra.len() > sunrise_protocol::menu_extra::MAX_DATA {
            return Err(OpenMenuError::TooLarge);
        }
        // A live handle of a connected player; a fake player has no connection.
        self.online_player_record(player)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let conn = self
            .native_conn(player.uuid())
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let guest = self.lan.as_ref().and_then(|host| host.hub.guest(conn));
        let brand = guest.map_or("", |g| g.brand.as_str());
        if !sunrise_client::containers::menu_reaches(brand, &id)
            // An older Sunrise client reads no menu opened without a block: its
            // legacy menu-extra layout has no way to carry "no position".
            || guest.is_some_and(|g| g.sender.menu_extra_rev == 0)
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        // The mods may stop the menu, as for a block's.
        if !self.guest_may_open(conn, &id, pos) {
            return Ok(None);
        }
        self.close_for_open(conn, replace);
        let pos = pos.map(|[x, y, z]| (x, y, z));
        let hidden = self.native_hidden_slots(&id, pos, kind.slots);
        let gi = self
            .guest_index(conn)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        // The first contents show the player's own inventory and cursor, in their turn too.
        let window = self.with_own_record(gi, |lan, _| {
            let window = lan.hub.open_menu_window(
                conn,
                &id,
                component_nbt(&title.component()),
                pos,
                sunrise_client::lan::menu::Slots::Native,
                vec![None; kind.slots],
                Some(extra),
            );
            if window.is_some() && !hidden.is_empty() {
                lan.hub.hide_menu_slots(conn, hidden);
            }
            window
        });
        if window.is_some() {
            self.player_menu_opening(conn);
        }
        Ok(window)
    }
    /// The native menu the player has open as window `window`, for its slots;
    /// None for another window, a menu the game opened, or a player not here.
    pub(super) fn menu(&self, uuid: u128, window: i32) -> Option<MenuRef> {
        let conn = self.native_conn(uuid)?;
        self.lan
            .as_ref()?
            .hub
            .guest(conn)?
            .menu
            .as_ref()
            .filter(|m| {
                m.window == window && m.slots == sunrise_client::lan::menu::Slots::Native
            })?;
        self.player_container_menu(self.player(uuid)?)
            .filter(|menu| menu.fields().window == window)
    }
    /// Replaces what the player's open native menu `window` shows, without reopening it
    /// (`AbstractContainerMenu.sendAllDataToRemote`): the whole contents in one packet.
    pub(super) fn menu_set_items(
        &mut self,
        uuid: u128,
        window: i32,
        items: Vec<Option<ItemStack>>,
    ) -> Result<(), mod_api::MenuItemsRefusal> {
        use mod_api::MenuItemsRefusal;
        if self.remote.is_some() {
            return Err(MenuItemsRefusal::Client);
        }
        let conn = self.native_conn(uuid).ok_or(MenuItemsRefusal::NoPlayer)?;
        let host = self.lan.as_mut().ok_or(MenuItemsRefusal::NoPlayer)?;
        if host.hub.set_native_menu_items(conn, window, items) {
            Ok(())
        } else {
            Err(MenuItemsRefusal::NotOpen)
        }
    }
    /// Closes the player's native menu (a `native_menu_closed` follows).
    pub(super) fn close_menu(&mut self, uuid: u128) {
        if let Some(conn) = self.native_conn(uuid)
            && let Some(host) = self.lan.as_mut()
            && host
                .hub
                .guest(conn)
                .and_then(|g| g.menu.as_ref())
                .is_some_and(|m| m.slots == sunrise_client::lan::menu::Slots::Native)
        {
            self.guest_close_menus_and_tell(conn);
        }
    }
}

type ContainerPos = (i32, i32, i32);

/// A menu the player opened, including the block entity instances it opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HeardMenu {
    window: i32,
    /// The menu type's name; a mount's inventory has none in vanilla.
    menu: String,
    /// The block the menu is of.
    pos: Option<[i32; 3]>,
    /// Each position and its generation when the menu opened; combined menus hold both halves.
    openers: [Option<(ContainerPos, u64)>; 2],
}

impl GuestMenus {
    pub(super) fn menu_holds(
        &self,
        menu: Option<&Menu>,
        pos: ContainerPos,
        generation: u64,
    ) -> bool {
        self.menu_heard
            .as_ref()
            .zip(menu)
            .is_some_and(|(opened, live)| {
                opened.window == live.window && opened.openers.contains(&Some((pos, generation)))
            })
    }
}

/// What a mount's inventory is called, as vanilla's `HorseInventoryMenu` has
/// no menu type.
pub(super) const MOUNT_MENU: &str = "sunrise:mount_inventory";

/// The menu `conn` has open, whichever way the game keeps it: a block's
/// container, a trade, a crafting table, a workstation, a lectern or a
/// mount's inventory. Not the player's own inventory menu.
pub(super) fn open_menu_of(host: &LanHost, conn: u32) -> Option<(i32, &str, Option<[i32; 3]>)> {
    let g = host.hub.guest(conn)?;
    let play = host.play.get(&conn);
    let menus = host.menus.get(&conn);
    if let Some(m) = &g.menu {
        let menu = host.hub.registry_name("minecraft:menu", m.kind)?;
        // A vehicle's menu is keyed by a stand-in, not a block.
        let pos = m
            .pos
            .filter(|p| p.1 != sunrise_client::lan::menu::VEHICLE_KEY_Y)
            .map(|p| [p.0, p.1, p.2]);
        return Some((m.window, menu, pos));
    }
    let play = play?;
    let (window, menu, pos) = if let Some(gm) = &play.merchant {
        (gm.window, "minecraft:merchant", None)
    } else if let Some((window, _, _, pos)) = menus.and_then(|m| m.crafting.table.as_ref()) {
        (*window, "minecraft:crafting", Some(*pos))
    } else if let Some(st) = menus.and_then(|m| m.station.as_ref()) {
        (
            st.window,
            super::lan_stations::menu_of(st.kind)?.0,
            Some(st.pos),
        )
    } else if let Some(l) = &play.lectern {
        (
            l.window,
            "minecraft:lectern",
            Some([l.key.0, l.key.1, l.key.2]),
        )
    } else if let Some(m) = &play.mount_window {
        (m.window, MOUNT_MENU, None)
    } else {
        return None;
    };
    Some((window, menu, pos))
}

/// Where the game calls the menu hook points.
impl Game<'_> {
    /// `ServerPlayer.openMenu` / `openHorseInventory`: called only after
    /// constructing the menu, queuing its open packet and sending full contents.
    /// Remember it before dispatch so a hook can close it synchronously.
    pub(super) fn player_menu_opening(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some((window, menu, pos)) = open_menu_of(host, conn) else {
            return;
        };
        let openers = host
            .hub
            .guest(conn)
            .and_then(|g| g.menu.as_ref())
            .map_or([None; 2], |m| {
                let positions = match m.slots {
                    Slots::Block(at) => [Some(at), None],
                    Slots::Double(first, second) => [Some(first), Some(second)],
                    Slots::Ender => [m.pos, None],
                    Slots::Native => [None; 2],
                };
                positions.map(|pos| {
                    pos.and_then(|pos| {
                        self.world
                            .block_entities
                            .generation(&pos)
                            .map(|generation| (pos, generation))
                    })
                })
            });
        let menu = HeardMenu {
            window,
            menu: menu.to_owned(),
            pos,
            openers,
        };
        host.menus.entry(conn).or_default().menu_heard = Some(menu.clone());
        self.player_menu_changed(conn, menu, true);
    }

    /// `conn`'s tracked menu closes with the guest's state in place
    /// (`ServerPlayer.doCloseContainer`). Close listeners are checked independently
    /// of open listeners.
    pub(super) fn player_menu_closing(&mut self, conn: u32) {
        let heard = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| p.menu_heard.take());
        if let Some(old) = heard {
            self.player_menu_changed(conn, old, false);
        }
    }

    fn player_menu_changed(&mut self, conn: u32, menu: HeardMenu, open: bool) {
        let hook = if open {
            mod_api::HookId::PlayerContainerOpen
        } else {
            mod_api::HookId::PlayerContainerClose
        };
        let heard = mod_api::__private::listened(hook);
        let openers = menu
            .openers
            .map(|opener| opener.filter(|&(pos, _)| self.mod_menu_at(pos).is_some()));
        if !heard && openers == [None; 2] {
            return;
        }
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return;
        };
        let uuid = guest.uuid;
        let name = heard.then(|| guest.name.clone());
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            for (pos, generation) in openers.into_iter().flatten() {
                g.mod_container_player_opener_for(pos, open, Some(generation));
            }
            if let Some(name) = name {
                if open {
                    let handle = g
                        .player(uuid)
                        .and_then(|player| g.player_container_menu(player));
                    g.native_player_container_open(
                        uuid, &name, dimension, &menu.menu, menu.pos, handle,
                    );
                } else {
                    g.native_player_container_close(uuid, &name, dimension, &menu.menu, menu.pos);
                }
            }
        });
    }

    /// Runs a player hook with the guest's state in place, or as it is when
    /// the guest has no body to swap in.
    fn player_hook(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        let mut f = Some(f);
        if self.lan_acting != Some(conn) {
            self.as_guest(conn, |g| {
                if let Some(f) = f.take() {
                    f(g);
                }
            });
        }
        if let Some(f) = f {
            f(self);
        }
    }

    /// The clicks on and closings of native menus the hub collected, each
    /// told to the mods with the hub in `self.lan`, no player's state
    /// swapped in.
    pub(super) fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::Click {
                    uuid,
                    window,
                    slot,
                    button,
                    mode,
                } => {
                    // A click a mod's backed slots answer (#3385); any other goes to the mods,
                    // as a decided action when a listener is subscribed (#784 3b).
                    if !self.place_menu_click(uuid, window, slot, button, mode) {
                        self.menu_clicked_decided(uuid, window, slot, button, mode);
                    }
                }
                NativeMenuEvent::Closed { uuid, window } => {
                    self.menu_places_closed(uuid, window);
                    self.native_menu_closed(uuid, window);
                }
                // A write to a backed menu's storage (#3385): its slots looked at again.
                NativeMenuEvent::PlacesChanged { uuid, window } => {
                    self.menu_places_changed(uuid, window)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use mod_api::{HostTransaction, ItemPlace, ItemResource, ServerHost};
    use std::sync::{Arc, mpsc};
    use sunrise_client::lan::{Out, PlayerSender};
    use sunrise_protocol::{
        buf::Reader,
        packets::play::{self, Clientbound as C, Serverbound as S},
    };

    /// Guest 1 (UUID 123) on a world open to LAN, and what their client is sent.
    fn watched_guest(game: &mut Game<'_>) -> mpsc::Receiver<Out> {
        open_to_lan(game);
        let (tx, rx) = mpsc::channel();
        let mut player = guest(1);
        player.sender = PlayerSender::new(tx, Arc::default());
        game.lan.as_mut().unwrap().hub.guests.push(player);
        rx
    }

    /// The packets sent since the last call.
    fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                C::decode(id, reader.rest()).unwrap()
            })
            .collect()
    }

    fn open(game: &mut Game<'_>, rows: u8, title: &str, replace: MenuReplace) -> i32 {
        let host: &mut dyn ServerHost = game;
        host.open_menu(
            123,
            rows,
            mod_api::Text::literal(title),
            Vec::new(),
            replace,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn acting_menu_storage_extraction_rolls_back() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let stack = ItemStack::new("minecraft:diamond", 5);
                let resource = ItemResource::of(&stack.clone().try_into_api().unwrap());
                let mut restored = Vec::new();
                for ender in [true, false] {
                    let host = game.lan.as_mut().unwrap();
                    if ender {
                        host.hub
                            .guests
                            .update(0, |g| g.ender_items[0] = Some(stack.clone()));
                        host.hub.open_container(
                            1,
                            "minecraft:generic_9x3",
                            sunrise_core::nbt::Tag::String("ender".into()),
                            (0, 64, 0),
                            sunrise_client::lan::menu::Slots::Ender,
                            vec![Some(stack.clone())],
                            None,
                        );
                    } else {
                        host.hub
                            .open_native_menu(
                                1,
                                3,
                                sunrise_core::nbt::Tag::String("native".into()),
                                vec![Some(stack.clone())],
                            )
                            .unwrap();
                    }
                    game.as_guest(1, |game| {
                        let menu = game
                            .player_container_menu(game.player(123).unwrap())
                            .unwrap();
                        let place = ItemPlace::MenuSlot { menu, slot: 0 };
                        let mut inner_restored = None;
                        let host: &mut dyn ServerHost = game;
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), 1);
                            let inner = host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), 1);
                                Err::<(), ()>(())
                            });
                            assert_eq!(inner, Err(()));
                            inner_restored = place.stack(host);
                            assert_eq!(place.extract(host, tx, &resource, 1), 1);
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(()));
                        restored.push((inner_restored, place.stack(host)));
                        // Player and menu writes share the ender snapshot, not competing copies.
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), 1);
                            assert_eq!(menu.cursor().insert(host, tx, &resource, 1), 1);
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(()));
                        assert_eq!(
                            place.stack(host),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                        assert_eq!(menu.carried(host), Some(None));
                        // A committed child lends its first snapshot to the parent.
                        let aborted = host.transaction(|host, _| {
                            host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), 1);
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(()));
                        assert_eq!(
                            place.stack(host),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                    });
                    game.guest_close_all(1);
                }
                let expected = |n| {
                    Some(
                        ItemStack::new("minecraft:diamond", n)
                            .try_into_api()
                            .unwrap(),
                    )
                };
                assert_eq!(
                    restored,
                    vec![(expected(4), expected(5)), (expected(4), expected(5))]
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn container_open_handles_follow_supported_menus() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        struct Watch(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                match event.menu_type {
                    "minecraft:furnace" => {
                        let menu = event.menu.expect("furnace has a handle during open");
                        assert_eq!(
                            menu.slot(host, 3),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ingot", 5)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                        assert_eq!(
                            menu.slot(host, 0),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ore", 3)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                    }
                    "minecraft:crafting" => assert!(event.menu.is_none()),
                    "minecraft:generic_9x3" => assert!(event.menu.is_some()),
                    other => panic!("unexpected menu {other}"),
                }
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new("watch", Box::new(Watch(events.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _rx = watched_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.inventory[9] = Some(ItemStack::new("minecraft:iron_ingot", 5));
                });
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                for name in [
                    "minecraft:chest",
                    "minecraft:furnace",
                    "minecraft:crafting_table",
                ] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    assert_eq!(
                        game.level_set_block(
                            level,
                            [0, 64, 0],
                            state,
                            mod_api::BlockUpdateFlags::NEIGHBORS
                        ),
                        Ok(true)
                    );
                    if name == "minecraft:furnace" {
                        let Some(super::super::BlockEntity::Furnace(furnace)) =
                            game.world.block_entities.get_mut(&(0, 64, 0))
                        else {
                            panic!("furnace block entity")
                        };
                        furnace.items[0] = Some(ItemStack::new("minecraft:iron_ore", 3));
                    }
                    if name == "minecraft:crafting_table" {
                        game.lan_open_crafting(1, [0, 64, 0]);
                    } else {
                        assert!(game.lan_open_container(1, (0, 64, 0)));
                    }
                    // The same accessor works in any acting server turn, not only the notice.
                    game.as_guest(1, |game| {
                        let player = game.player(123).unwrap();
                        let menu = game.player_container_menu(player);
                        if name == "minecraft:crafting_table" {
                            assert!(menu.is_none());
                        } else {
                            let menu = menu.unwrap();
                            assert!(game.menu_slot_count(menu).is_some());
                            // The cursor is swapped out of Guest for this turn.
                            let cursor = Some(ItemStack::new("minecraft:stone", 2));
                            game.carried = cursor.clone();
                            assert_eq!(game.menu_carried(menu), Some(cursor));
                            game.carried = None;
                            let fields = menu.fields();
                            let stale = mod_api::__private::game::menu_ref(
                                player,
                                fields.window,
                                fields.generation + 1,
                            );
                            assert_eq!(game.menu_slot(stale, 0), None);
                            let wrong_window = mod_api::__private::game::menu_ref(
                                player,
                                fields.window + 1,
                                fields.generation,
                            );
                            assert_eq!(game.menu_slot(wrong_window, 0), None);
                        }
                    });
                    game.guest_close_all(1);
                }
                assert_eq!(
                    *events.lock().unwrap(),
                    [
                        "open minecraft:generic_9x3",
                        "close minecraft:generic_9x3",
                        "open minecraft:furnace",
                        "close minecraft:furnace",
                        "open minecraft:crafting",
                        "close minecraft:crafting",
                    ]
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn container_hooks_follow_actual_open_boundaries_without_a_tick() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::container_hooks_follow_actual_open_boundaries_without_a_tick",
            "SUNRISE_CONTAINER_BOUNDARY_CASE",
            &["same_tick", "replace", "mount", "guest"],
        ) {
            return;
        }
        struct Watch {
            events: Arc<Mutex<Vec<(bool, String)>>>,
            packets: Arc<Mutex<Option<mpsc::Receiver<Out>>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                // Both packets must precede the callback, not just the open's return.
                let packets = self.packets.lock().unwrap();
                let packets = sent(packets.as_ref().unwrap());
                let open = packets
                    .iter()
                    .find_map(|packet| match packet {
                        C::OpenScreen(p) => Some(p.container),
                        C::MountScreenOpen(p) => Some(p.container),
                        _ => None,
                    })
                    .unwrap();
                assert!(packets.iter().any(|packet| {
                    matches!(packet, C::ContainerSetContent(p) if p.container == open)
                }));
                self.events
                    .lock()
                    .unwrap()
                    .push((true, event.menu_type.to_string()));
            }
            fn player_container_close(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.events
                    .lock()
                    .unwrap()
                    .push((false, event.menu_type.to_string()));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let packets = Arc::new(Mutex::new(None));
        let mods: Mods = vec![ModEntry::new(
            "watch",
            Box::new(Watch {
                events: events.clone(),
                packets: packets.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            *packets.lock().unwrap() = Some(watched_guest(game));
            let case = std::env::var("SUNRISE_CONTAINER_BOUNDARY_CASE").unwrap();
            // Own inventory and unsuccessful construction have no open boundary.
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 0 }));
            game.lan_open_mount(1, u32::MAX);
            assert!(!game.lan_open_container(1, (0, 64, 0)));
            assert!(events.lock().unwrap().is_empty());
            if case == "mount" {
                game.mobs.push(sunrise_client::mobs::Mob::new(
                    31,
                    game.res.mob_types().get("minecraft:horse").unwrap().clone(),
                    sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
                    0.0,
                ));
                game.lan_open_mount(1, 31);
                assert_eq!(
                    open_menu_of(game.lan.as_ref().unwrap(), 1),
                    Some((1, "sunrise:mount_inventory", None))
                );
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "sunrise:mount_inventory".into()),
                        (false, "sunrise:mount_inventory".into()),
                    ]
                );
                return;
            }
            let first = if case == "guest" {
                let mut window = None;
                game.as_guest(1, |game| {
                    window = Some(open(game, 1, "first", MenuReplace::CloseFirst));
                });
                window.unwrap()
            } else {
                open(game, 1, "first", MenuReplace::CloseFirst)
            };
            assert_eq!(first, 1);
            assert_eq!(
                open_menu_of(game.lan.as_ref().unwrap(), 1),
                Some((1, "minecraft:generic_9x1", None))
            );
            assert_eq!(
                *events.lock().unwrap(),
                vec![(true, "minecraft:generic_9x1".into())]
            );
            if case == "replace" {
                assert_eq!(open(game, 2, "second", MenuReplace::InPlace), 2);
                // A stale close must not notify the replacement.
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                    ]
                );
                game.close_menu(123);
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                        (false, "minecraft:generic_9x2".into()),
                    ]
                );
            } else {
                game.lan_guest_packet(
                    1,
                    S::ContainerClose(play::ContainerClose { container: first }),
                );
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                    ]
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_slot_written_outside_a_click_is_one_set_slot_at_the_next_sync() {
        with_game(|game| {
            let rx = watched_guest(game);
            let window = open(game, 1, "shop", MenuReplace::CloseFirst);
            assert_eq!(window, 1);
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 2), None, "another window");
            assert_eq!(host.menu(124, 1), None, "another player");
            let menu = host.menu(123, 1).unwrap();
            assert_eq!(menu.container_id(host), Some(1));
            sent(&rx);

            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            let slot = ItemPlace::MenuSlot { menu, slot: 4 };
            let written = host.transaction(|host, tx| {
                Ok::<_, ()>(slot.insert(host, tx, &ItemResource::of(&diamond), 3))
            });
            assert_eq!(written, Ok(3));
            assert!(sent(&rx).is_empty(), "nothing before the tick's sync");

            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "{packets:?}");
            let C::ContainerSetSlot(set) = &packets[0] else {
                panic!("{packets:?}");
            };
            // The open's content took state 1; this is the next.
            assert_eq!((set.container, set.state, set.slot), (1, 2, 4));
            assert_eq!(set.item.as_ref().map(|item| item.count), Some(3));
            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            assert!(sent(&rx).is_empty(), "sent once");

            // Closed, the menu answers nothing.
            game.close_menu(123);
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 1), None);
        });
    }

    /// `menu_set_items` sends the whole contents of the open menu in one packet for its window,
    /// padded to its slots and cut at them, and the screen, the title and the cursor stay.
    #[test]
    #[ignore = "daily"]
    fn a_menus_items_are_replaced_in_one_packet_without_reopening() {
        with_game(|game| {
            let rx = watched_guest(game);
            let window = open(game, 1, "shop", MenuReplace::CloseFirst);
            let stone = ItemStack::new("minecraft:stone", 3);
            let cursor = ItemStack::new("minecraft:dirt", 2);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(cursor.clone()));
            sent(&rx);
            let api = |stack: &ItemStack| stack.clone().try_into_api().unwrap();
            let host: &mut dyn ServerHost = game;
            // Two stacks, then enough empty slots to be cut at the menu's nine.
            let mut items = vec![None, Some(api(&stone))];
            items.resize(12, None);
            items[11] = Some(api(&stone));
            assert_eq!(host.menu_set_items(123, window, items), Ok(()));
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.native_menu.len(), 9, "cut at the menu's slots");
            assert_eq!(guest.native_menu[1], Some(stone.clone()));
            assert_eq!(
                guest.native_menu[8], None,
                "the tenth to twelfth are dropped"
            );
            assert_eq!(guest.carried, Some(cursor), "the cursor stays");
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "one packet: {packets:?}");
            let C::ContainerSetContent(content) = &packets[0] else {
                panic!("{packets:?}");
            };
            assert_eq!(content.container, window);
            assert_eq!(content.items.len(), 9 + 36, "the menu, then the inventory");
            assert_eq!(content.items[1].as_ref().map(|item| item.count), Some(3));
            assert!(
                content.items[0].is_none(),
                "padded and empty slots are empty"
            );
            assert!(content.carried.is_some(), "the cursor is sent with it");
            let host: &mut dyn ServerHost = game;
            assert!(host.menu(123, window).is_some(), "still the open menu");
        });
    }

    /// A menu opened in the player's own turn, nested in another player's, clears and shows
    /// that player's own cursor and inventory: none of the outer player's stacks reach them.
    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_a_nested_turn_sends_the_openers_own_inventory_and_cursor() {
        with_game(|game| {
            let rx = watched_guest(game);
            let mut outer = guest(2);
            outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
            outer.carried = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.inventory[5] = Some(ItemStack::new("minecraft:gold_ingot", 2));
                g.carried = Some(ItemStack::new("minecraft:stone", 4));
            });
            sent(&rx);
            let window = game
                .as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "pack", MenuReplace::CloseFirst))
                })
                .flatten()
                .unwrap();
            let contents: Vec<_> = sent(&rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::ContainerSetContent(content) => Some(content),
                    _ => None,
                })
                .collect();
            // The cursor's stone back in the inventory, then the menu's first contents.
            let windows: Vec<_> = contents.iter().map(|c| c.container).collect();
            assert_eq!(windows, [0, window]);
            for content in &contents {
                let mut counts: Vec<_> = content.items.iter().flatten().map(|s| s.count).collect();
                counts.sort_unstable();
                assert_eq!(
                    counts,
                    [2, 4],
                    "window {}: their gold and stone",
                    content.container
                );
                assert!(content.carried.is_none(), "window {}", content.container);
            }
            let hub = &game.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried, None);
            assert_eq!(
                hub.guests[1].carried,
                Some(ItemStack::new("minecraft:dirt", 9)),
                "the outer player's cursor stays theirs"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cursor_overflow_listeners_see_and_write_only_the_openers_inventory() {
        use mod_api::{
            EntityJoinLevelEvent, Flow, PlayerContainerCloseEvent, PlayerContainerOpenEvent,
        };
        use std::sync::Mutex;

        struct Listen(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:item" {
                    let opener = host.player(123).unwrap();
                    let stone = ItemStack::new("minecraft:stone", 64)
                        .try_into_api()
                        .unwrap();
                    assert_eq!(
                        opener.inventory_slots(host),
                        Some((0..36).map(|slot| (slot, stone.clone())).collect()),
                        "the listener sees the opener's full inventory after cursor reinsertion"
                    );
                    opener.set_inventory_slot(host, 3, None).unwrap();
                    self.0.lock().unwrap().push("drop");
                }
                Flow::Continue
            }

            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0.lock().unwrap().push("close");
            }

            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0.lock().unwrap().push("open");
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "overflow-listener",
            Box::new(Listen(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let mut outer = guest(2);
                outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
                let outer_inventory = outer.inventory.clone();
                game.lan.as_mut().unwrap().hub.guests.push(outer);
                open(game, 1, "old", MenuReplace::CloseFirst);
                heard.lock().unwrap().clear();
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)));
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 62));
                    g.carried = Some(ItemStack::new("minecraft:stone", 4));
                });
                let dropped = game.dropped.len();
                game.as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "new", MenuReplace::CloseFirst))
                        .unwrap();
                })
                .unwrap();
                assert_eq!(*heard.lock().unwrap(), ["drop", "close", "open"]);
                assert_eq!(game.dropped.len(), dropped + 1);
                assert_eq!(
                    game.dropped[dropped].stack,
                    ItemStack::new("minecraft:stone", 2)
                );
                let guests = &game.lan.as_ref().unwrap().hub.guests;
                assert_eq!(
                    guests[0].inventory[3], None,
                    "the listener's write belongs to the opener"
                );
                assert_eq!(guests[1].inventory, outer_inventory);
            });
        });
    }

    /// Each refusal has its own name and sends and changes nothing: another window or none
    /// open, no such player, a stack the game refuses, and a client.
    #[test]
    #[ignore = "daily"]
    fn each_menu_items_refusal_has_its_own_name_and_sends_nothing() {
        use mod_api::{MenuItemsRefusal, StackWriteError};
        with_game(|game| {
            let rx = watched_guest(game);
            let api = |stack: ItemStack| stack.try_into_api().unwrap();
            let one = vec![Some(api(ItemStack::new("minecraft:stone", 1)))];
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.menu_set_items(123, 1, one.clone()),
                Err(MenuItemsRefusal::NotOpen),
                "no menu open"
            );
            let window = open(game, 1, "shop", MenuReplace::CloseFirst);
            sent(&rx);
            let before = game.lan.as_ref().unwrap().hub.guests[0].native_menu.clone();
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.menu_set_items(123, window + 1, one.clone()),
                Err(MenuItemsRefusal::NotOpen),
                "another window"
            );
            assert_eq!(
                host.menu_set_items(999, window, one.clone()),
                Err(MenuItemsRefusal::NoPlayer)
            );
            let mut too_many = api(ItemStack::new("minecraft:stone", 1));
            too_many.set_count(65).unwrap();
            assert_eq!(
                host.menu_set_items(123, window, vec![Some(too_many)]),
                Err(MenuItemsRefusal::Stack(StackWriteError::CountExceeded {
                    maximum: 64
                }))
            );
            assert!(sent(&rx).is_empty(), "nothing was sent");
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].native_menu,
                before,
                "nothing was changed"
            );
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "MenuItemsRefusalTest").unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                host.menu_set_items(123, window, one),
                Err(MenuItemsRefusal::Client)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_place_sends_no_close_returns_the_cursor_and_ignores_the_old_window() {
        with_game(|game| {
            let rx = watched_guest(game);
            let closes = |packets: &[C]| -> Vec<i32> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect()
            };
            let opens = |packets: &[C]| -> Vec<(i32, sunrise_core::nbt::Tag)> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::OpenScreen(open) => Some((open.container, open.title.clone())),
                        _ => None,
                    })
                    .collect()
            };
            // From the inventory nothing is open, so nothing closes.
            assert_eq!(open(game, 1, "one", MenuReplace::CloseFirst), 1);
            let packets = sent(&rx);
            assert_eq!(closes(&packets), Vec::<i32>::new());
            assert_eq!(opens(&packets).len(), 1);

            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            assert_eq!(open(game, 2, "two", MenuReplace::InPlace), 2);
            let packets = sent(&rx);
            assert_eq!(
                closes(&packets),
                Vec::<i32>::new(),
                "no close: the mouse stays"
            );
            assert_eq!(
                opens(&packets),
                vec![(2, component_nbt(&Text::literal("two").component()))]
            );
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 18)));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                vec![NativeMenuEvent::Closed {
                    uuid: 123,
                    window: 1
                }],
                "the old menu closed on the server"
            );

            // The old window's click and close land on nothing.
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            game.lan_guest_packet(
                1,
                S::ContainerClick(play::ContainerClick {
                    container: 1,
                    state: 1,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                Vec::new()
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(host.menu(123, 1), None);
            assert!(host.menu(123, 2).is_some(), "the new menu stays open");

            // The default sends the close first.
            sent(&rx);
            assert_eq!(open(game, 3, "three", MenuReplace::CloseFirst), 3);
            let packets = sent(&rx);
            assert_eq!(closes(&packets), vec![2]);
            assert_eq!(opens(&packets).len(), 1);
            let close_at = packets
                .iter()
                .position(|p| matches!(p, C::ContainerClose(_)))
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| matches!(p, C::OpenScreen(_)))
                .unwrap();
            assert!(close_at < open_at);
        });
    }

    /// The content mod menu type the mod-menu tests open: a prepared registry slot
    /// (`minecraft:generic_9x1`) for a synthetic definition.
    const MOD_MENU: &str = "minecraft:generic_9x1";

    /// [`with_game`] with [`MOD_MENU`] as the thread's menu types, so a test neither depends on
    /// registering first nor touches the one-shot process table.
    fn with_mod_menu_game(test: impl FnOnce(&mut Game<'_>)) {
        sunrise_client::containers::with_mod_menu_types(
            [(MOD_MENU.into(), 9, 0, Vec::new(), None, None)],
            || with_game(test),
        );
    }

    /// [`watched_guest`] on a Sunrise client that reads a menu opened with no block, and the
    /// key of [`MOD_MENU`].
    fn mod_menu_guest(game: &mut Game<'_>) -> (mpsc::Receiver<Out>, mod_api::MenuKey) {
        let rx = watched_guest(game);
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.brand = "sunrise".into();
            g.sender.menu_extra_rev = 1;
        });
        let key = mod_api::__private::menu_keys()
            .into_iter()
            .find(|key| mod_api::__private::menu_key_id(*key).as_deref() == Some(MOD_MENU))
            .unwrap();
        (rx, key)
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu() {
        use mod_api::{PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        const CASE: &str = "SUNRISE_MOD_MENU_REPLACE_CHILD";
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu",
            CASE,
            &["close_first", "in_place"],
        ) {
            return;
        }
        // What the mods heard, in order.
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct Listen;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let heard = || std::mem::take(&mut *HEARD.lock().unwrap());
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("listen", Box::new(Listen))].into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let replace = match std::env::var(CASE).unwrap().as_str() {
            "close_first" => MenuReplace::CloseFirst,
            _ => MenuReplace::InPlace,
        };
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            // Where the game keeps what the mods last heard the player open.
            game.lan.as_mut().unwrap().play.entry(1).or_default();
            let player = game.player(123).unwrap();
            assert_eq!(open(game, 3, "old", MenuReplace::CloseFirst), 1);
            assert_eq!(heard(), ["open minecraft:generic_9x3"]);
            sent(&rx);
            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));

            let window = game
                .player_open_mod_menu(player, key, Text::literal("new"), vec![7], None, replace)
                .unwrap()
                .unwrap();
            assert_eq!(window, 2);
            // The old menu's close, heard once, before anything of the new one.
            assert_eq!(
                heard(),
                [
                    "close minecraft:generic_9x3".to_owned(),
                    format!("open {MOD_MENU}")
                ]
            );
            let packets: Vec<String> = sent(&rx)
                .iter()
                .map(|p| match p {
                    C::ContainerClose(close) => format!("close {}", close.container),
                    C::CustomPayload(payload) => format!("payload {}", payload.channel),
                    C::OpenScreen(open) => format!("open {}", open.container),
                    C::ContainerSetContent(content) => format!(
                        "content {} state {} slots {}",
                        content.container,
                        content.state,
                        content.items.len()
                    ),
                    other => format!("{other:?}"),
                })
                .collect();
            // The client's close of the old window for `CloseFirst` only (queued before the
            // removal, as `closeContainer` does), the inventory with the cursor stack back in
            // it, then the new menu's extra data, screen and contents.
            let mut expected = vec![
                "content 0 state 1 slots 46",
                "payload sunrise:menu_extra",
                "open 2",
                "content 2 state 1 slots 45",
            ];
            if replace == MenuReplace::CloseFirst {
                expected.insert(0, "close 1");
            }
            assert_eq!(packets, expected);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 9)));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                vec![NativeMenuEvent::Closed {
                    uuid: 123,
                    window: 1
                }],
                "the old menu closed on the server"
            );

            // Both notifications were delivered inline, without a second close.
            assert!(heard().is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_menu_backend_payload_and_refusals() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::mod_menu_backend_payload_and_refusals",
            "SUNRISE_MOD_MENU_BACKEND_CHILD",
            &["backend"],
        ) {
            return;
        }
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            let player = game.player(123).unwrap();
            let old = open(game, 1, "old", MenuReplace::CloseFirst);
            sent(&rx);
            let bytes = vec![0, 255, 17];
            let window = game
                .player_open_mod_menu(
                    player,
                    key,
                    Text::literal("new"),
                    bytes.clone(),
                    None,
                    MenuReplace::CloseFirst,
                )
                .unwrap()
                .unwrap();
            assert_eq!(window, old + 1);
            let packets = sent(&rx);
            let payload_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL
                    )
                })
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::OpenScreen(p) if p.container == window
                    )
                })
                .unwrap();
            assert!(payload_at < open_at, "extra data must precede its window");
            let C::CustomPayload(payload) = &packets[payload_at] else {
                unreachable!()
            };
            assert_eq!(
                sunrise_protocol::menu_extra::decode(&payload.data).unwrap(),
                sunrise_protocol::menu_extra::Message {
                    window,
                    pos: None,
                    data: bytes
                }
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| (m.window, m.pos)),
                Some((window, None))
            );

            for (revision, extra, expected) in [
                (1, vec![0; 33 * 1024], mod_api::OpenMenuError::TooLarge),
                (0, vec![1], mod_api::OpenMenuError::PlayerUnavailable),
            ] {
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender.menu_extra_rev = revision;
                });
                game.lan.as_mut().unwrap().hub.native_menu_events.clear();
                assert_eq!(
                    game.player_open_mod_menu(
                        player,
                        key,
                        Text::literal("refused"),
                        extra,
                        None,
                        MenuReplace::InPlace,
                    ),
                    Err(expected)
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .map(|m| m.window),
                    Some(window),
                    "refusal preserves the current menu"
                );
                assert!(game.lan.as_ref().unwrap().hub.native_menu_events.is_empty());
                assert!(sent(&rx).is_empty(), "refusal sends neither close nor open");
            }
        });
    }

    #[test]
    fn menu_clicks_map_to_the_inventory_below() {
        // A 6-row chest: 54 slots, then the inventory's 27, then the hotbar.
        assert_eq!(native_menu_inventory_slot(54, 3), None);
        assert_eq!(native_menu_inventory_slot(54, 54), Some(9));
        assert_eq!(native_menu_inventory_slot(54, 80), Some(35));
        assert_eq!(native_menu_inventory_slot(54, 81), Some(0));
        assert_eq!(native_menu_inventory_slot(54, 89), Some(8));
        assert_eq!(native_menu_inventory_slot(54, 90), None);
        assert_eq!(native_menu_inventory_slot(54, -999), None);
        assert_eq!(native_menu_inventory_slot(9, 9), Some(9));
    }
}
