//! `PlayerRef::push_resource_pack` and `pop_resource_pack`: vanilla's
//! `ClientboundResourcePackPushPacket` and `ClientboundResourcePackPopPacket`, as
//! `/server_pack push` and `pop` send them, to one player's play connection.
use super::Game;
use mod_api::{PlayerRef, PlayerWorldRefusal, ResourcePackRefusal as Refusal};
use sunrise_client::{lan::data::component_nbt, text::Text};
use sunrise_protocol::packets::{
    configuration::{ResourcePackPop, ResourcePackPush},
    play::Clientbound as C,
};

/// `ClientboundResourcePackPushPacket.STREAM_CODEC`: the URL is a `STRING_UTF8` of at most
/// 32767 UTF-16 units, the hash a `stringUtf8(40)`.
const URL_UNITS: usize = 32767;
const HASH_UNITS: usize = 40;

impl Game<'_> {
    /// One player's play connection, or why it has none to send to.
    fn resource_pack_connection(&self, player: PlayerRef) -> Result<u32, Refusal> {
        self.player_connection(player)
            .map_err(|refusal| match refusal {
                PlayerWorldRefusal::Client => Refusal::Client,
                PlayerWorldRefusal::NoConnection => Refusal::NoConnection,
                _ => Refusal::Stale,
            })
    }

    fn send_resource_pack_packet(&self, conn: u32, packet: &C) -> Result<(), Refusal> {
        let guest = self
            .lan
            .as_ref()
            .and_then(|lan| lan.hub.guest(conn))
            .ok_or(Refusal::Stale)?;
        guest.send(packet);
        Ok(())
    }

    pub(super) fn player_push_resource_pack(
        &mut self,
        player: PlayerRef,
        id: u128,
        url: &str,
        sha1: &str,
        required: bool,
        prompt: Option<Text>,
    ) -> Result<(), Refusal> {
        let conn = self.resource_pack_connection(player)?;
        if url.encode_utf16().count() > URL_UNITS {
            return Err(Refusal::UrlTooLong);
        }
        if sha1.encode_utf16().count() > HASH_UNITS {
            return Err(Refusal::HashTooLong);
        }
        let packet = C::ResourcePackPush(ResourcePackPush {
            id,
            url: url.to_owned(),
            hash: sha1.to_owned(),
            required,
            prompt: prompt.map(|prompt| component_nbt(&prompt.component())),
        });
        self.send_resource_pack_packet(conn, &packet)
    }

    pub(super) fn player_pop_resource_pack(
        &mut self,
        player: PlayerRef,
        id: Option<u128>,
    ) -> Result<(), Refusal> {
        let conn = self.resource_pack_connection(player)?;
        self.send_resource_pack_packet(conn, &C::ResourcePackPop(ResourcePackPop { id }))
    }
}
