//! Registered raw payload sends; compatibility and pressure are connection-owned.
#![allow(unused_variables)]
use crate::*;

/// Player-targeted sends, exposed naturally through [`PlayerRef::send_payload`].
#[sunrise_mod_api_macros::host_area]
pub trait PayloadsBackendMut {
    /// Side: server.
    /// Queue exact application bytes for this live player's connection. The channel
    /// must be declared locally and negotiated for the current phase and direction.
    /// Publication waits for root commit; abort publishes nothing. Queued is not a
    /// delivery acknowledgement. No generic registered-body or per-tick count cap applies.
    fn player_send_payload(
        &mut self,
        player: PlayerRef,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError>;
}

/// The group send, a method of the server host: no one player is its receiver.
#[sunrise_mod_api_macros::host_area]
pub trait PayloadsServerHost {
    /// Side: server.
    /// Queue the same application bytes on each of `players`' connections: one crossing and one
    /// buffer for the whole group, where `player_send_payload` repeats both per player. Each
    /// send follows `player_send_payload`'s rules; a player whose send is refused (offline or
    /// stale, no channel, wrong phase, backpressure, framing) is skipped and not counted. Returns
    /// how many sends were accepted, which is 0 for an empty list; a player listed twice is sent
    /// to twice. Publication waits for root commit; abort publishes nothing.
    fn players_send_payload(
        &mut self,
        players: &[PlayerRef],
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> u32;
}

/// Client connection sends, available during configuration as well as play.
#[sunrise_mod_api_macros::host_area]
pub trait PayloadsClientHost {
    /// Side: client.
    /// Queue exact application bytes on a registered serverbound mod channel. The
    /// active connection must have negotiated the channel for its current phase
    /// (configuration or play). `Disconnected` means there is no live connection;
    /// `NotNegotiated` means it has no current peer table. Direction, channel,
    /// framing and backpressure refusals write no application bytes.
    ///
    /// Outside a transaction, accepted sends are submitted in call order without
    /// waiting for a client tick. Inside one, sends join its ordered journal: a
    /// child commit passes them to its parent, the root commit submits them, and
    /// an abort discards them. Pending sends never cross a connection or phase change.
    /// [`PayloadAccepted::Queued`] means accepted locally, not delivered.
    fn send_payload_to_server(
        &self,
        id: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError>;
}
