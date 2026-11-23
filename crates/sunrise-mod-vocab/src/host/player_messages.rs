//! The `player_messages` area: chat, action bar, tab list, sounds and maps sent to one player. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' writes of the `player_messages` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerMessagesBackendMut {
    /// Side: server.
    /// A system line in that player's chat, also inside a player's hook for
    /// the acting player; false when they aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_tell(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// A line over that player's hotbar (the action bar), also inside a
    /// player's hook for the acting player; false when they aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_action_bar(&mut self, player: PlayerRef, line: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// `ServerPlayer.openDialog` with an inline dialog: `snbt` is the dialog as a data pack
    /// file writes it (`type`, `title`, `body`, `inputs`, `actions`), in the SNBT form
    /// `/dialog show` takes. Only its syntax is checked, not the dialog's fields: a client
    /// that can't read it disconnects, as with `/dialog show`. Also inside a player's hook
    /// for the acting player, whose packet goes after the hook. False for a player who
    /// isn't on, a fake player, on a client, or text that isn't SNBT. `true` means the
    /// packet was accepted for sending, not that the dialog is shown.
    fn player_show_dialog(&mut self, player: PlayerRef, snbt: &str) -> bool;
    /// Side: server.
    /// `ServerPlayer.openDialog` with an entry of the `minecraft:dialog` registry (vanilla's,
    /// a data pack's or a mod's), `id` with or without the `minecraft:` namespace. False,
    /// sending nothing, for an id the registry lacks, and as
    /// [`Self::player_show_dialog`] is.
    fn player_show_dialog_by_id(&mut self, player: PlayerRef, id: &str) -> bool;
    /// Side: server.
    /// `/dialog clear` (`ClientboundClearDialogPacket`): the player's client closes the
    /// dialog it shows, and does nothing when none is shown. The server is never told when
    /// a player closes a dialog, so there is no read of whether one is open. False as
    /// [`Self::player_show_dialog`] is.
    fn player_clear_dialog(&mut self, player: PlayerRef) -> bool;
    /// Side: server.
    /// The lines over and under that player's list of players (Tab):
    /// `ClientboundTabListPacket`, an empty text for none. False when they
    /// aren't on.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    #[serialize_text]
    fn player_tab_list(
        &mut self,
        player: PlayerRef,
        header: &Text,
        footer: &Text,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    fn player_send_map(&mut self, player: PlayerRef, map_id: i32, colors: &[u8]) -> bool;
    /// Side: server.
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. `true` means the request was accepted, not that a client received or played it.
    /// Inside an open transaction, delivery waits for the owning level's outermost commit in
    /// journal order. A deferred request is discarded on abort or if the player no longer
    /// resolves at commit.
    fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool;
    /// Side: server.
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// request level's random `nextLong`, drawn at the call outside a transaction or when
    /// published at the owning level's outermost commit. `true` means the request was accepted,
    /// not that a client received or played it. A deferred request is discarded on abort or if
    /// the player no longer resolves at commit, without drawing a seed.
    fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
}
