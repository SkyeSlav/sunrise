//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `players` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersHost {
    /// Side: both.
    /// `ItemCooldowns.getCooldownGroup(stack)`: the group `stack`'s
    /// cooldown is filed under, the group its `use_cooldown` component names
    /// (its own, else its item's default), else its item id.
    #[access(Metadata)]
    fn item_cooldown_group(&self, stack: &ItemStack) -> String;
    /// Side: both.
    /// Resolve a real online player by UUID, never a fake. A fake is reached
    /// through the handle returned by its factory. During a nested player swap,
    /// outer players whose records are displaced return None.
    #[access(WorldOnly)]
    fn player(&self, uuid: u128) -> Option<PlayerRef>;
}
/// The server host's methods of the `players` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersServerHost {
    /// Side: server.
    /// Create or retrieve a connectionless player for the level currently in place.
    #[access(WorldOnly)]
    fn fake_player(&mut self, level: LevelRef, uuid: u128, name: &str) -> Option<PlayerRef>;
    /// Side: server.
    /// The most players who may be on at once (`PlayerList.getMaxPlayers`,
    /// `max-players`), as the server list shows it and logins are refused
    /// beyond. None where no server is open to players (a client, or a server
    /// nobody can join): the one reason there is none.
    #[access(WorldOnly)]
    fn server_max_players(&self) -> Option<u32>;
    /// Side: server.
    /// Sets the player limit from the next login and server list ping on
    /// (Paper's `Server.setMaxPlayers`); players already on stay, even when
    /// they are more than the new limit. `server.properties` is not changed,
    /// so a restart goes back to its `max-players`. Writes nothing and refuses
    /// with `PlayerLimitRefusal::NotOpen` where no server is open to players,
    /// and with `TooHigh { maximum }` for a limit above `i32::MAX`, as vanilla
    /// keeps the limit in an `int` (Paper's `setMaxPlayers` takes one). Zero is
    /// taken: it lets nobody else on.
    #[access(WorldOnly)]
    fn server_set_max_players(&mut self, max_players: u32) -> Result<(), PlayerLimitRefusal>;
}
/// The client host's methods of the `players` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientHost {
    /// Side: client.
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone.
    #[access(WorldOnly)]
    fn push_player(&mut self, kick: [f32; 3]);
    /// Side: client.
    /// For a client mod: chooses the local player's hotbar slot, `slot` (0 to 8), as
    /// `Inventory.setSelectedSlot` does for a number key. It sends nothing itself: the game
    /// sends the carried-item packet at the next tick's start at the latest, and before the
    /// packets of an attack, a use, an interact, a release, a drop and a continued block break
    /// (not before a block break start). Refused above 8 with `OutOfRange`, without a client
    /// world with `NoWorld`, and for a spectator with `Refused` (its number keys open the
    /// spectator menu), leaving the selection as it was.
    #[access(WorldOnly)]
    fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError>;
    /// Side: client.
    /// For a client mod: the local player's collision exclusions, the client half of
    /// `EntityRef::set_collision_exclusions`, with the same boxes, rules and refusals (a client
    /// with no player of its own answers `Stale`). The client moves its player itself, so a mod
    /// sets the same boxes on both sides: the engine sends neither set to the other.
    #[access(WorldOnly)]
    fn set_collision_exclusions(
        &mut self,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError>;
    /// Moves the window's player through `passage` (#3775), one its server told it of, as
    /// Immersive Portals' client does: its feet, where it was a tick ago (so the frames
    /// between two ticks do not jump), its velocity and its look come out at the far side,
    /// turned with the passage, and the server is told the passage and where the feet were.
    /// Nothing else of the player changes: on ground, fall distance, sprinting, what it
    /// uses. The server checks the move to there as any move, then answers with a position
    /// packet that changes nothing (all relative and zero), so the player keeps its velocity;
    /// as any position packet does, it stops mining (a passage crossed while mining aborts
    /// the dig). A refused crossing puts the player back where the server has it
    /// ([`Self::last_passage_refusal`] says why). One crossing at a time: the next waits
    /// for the server's answer. Refused, with nothing moved or sent, when not connected to
    /// a server, with no player, while riding, for a passage the window does not hold, when
    /// the feet are more than 20 blocks from the opening, and when a mod's
    /// `client_packet_send` cancels the crossing's payload.
    #[access(WorldOnly)]
    fn cross_passage(&mut self, passage: PassageId) -> Result<(), PassageCrossError>;
    /// Side: client.
    /// The server's refusal of the window's last crossing ([`Self::cross_passage`]) and the
    /// passage it named, until the next crossing; None when it was not refused or no answer
    /// came yet.
    #[access(WorldOnly)]
    fn last_passage_refusal(&self) -> Option<(PassageId, PassageCrossRefusal)>;
}
/// The handles' client reads of the `players` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientBackend {
    /// Side: client.
    /// The local player's last authoritative custom-book pair. None before
    /// negotiation, while resynchronizing, or for a nonlocal/stale handle.
    #[access(WorldOnly)]
    fn player_synced_recipe_book_settings(
        &self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
    ) -> Option<RecipeBookSettings>;
    /// Side: client.
    /// The local player's predicted breaking state, including Idle. None for
    /// stale, nonlocal or fake handles and outside a logged-in client world.
    #[access(WorldOnly)]
    fn player_block_breaking(&self, player: PlayerRef) -> Option<BlockBreakingState>;
    /// Side: client.
    /// The local player's existing cave ambience mood, normally 0 through 1.
    /// None for stale, nonlocal or fake handles and outside a client world.
    #[access(WorldOnly)]
    fn player_ambient_mood(&self, player: PlayerRef) -> Option<f32>;
}
/// Client-only custom book intent, reached through player handles.
// hidden: mods call PlayerRef methods, never the generated backend trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayersClientBackendMut {
    /// Side: client.
    /// Request a new pair for the local player's negotiated book. True means
    /// queued, not accepted; only an authoritative reply changes saved state.
    #[access(WorldOnly)]
    fn player_request_recipe_book_settings(
        &mut self,
        player: PlayerRef,
        book: &RecipeBookTypeKey,
        settings: RecipeBookSettings,
    ) -> bool;
}
