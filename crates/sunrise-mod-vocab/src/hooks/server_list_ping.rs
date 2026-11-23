//! The `server_list_ping` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before replying to the first modern status request on a connection, on
    /// the server thread. There is no acting player. Disabled status, legacy
    /// queries, duplicate requests and latency pings do not fire this hook.
    ///
    /// `protocol()` is the client's handshake protocol, not the advertised one.
    /// `virtual_host()` is the untrusted host string as decoded by
    /// `ClientIntentionPacket`, without its separate port.
    ///
    /// Edit the handshake's status snapshot for this response only. `motd` is
    /// structured text. `favicon` is PNG bytes, or `None` for no icon; invalid
    /// icons and icons other than 64 × 64 are omitted. `online` and `max_players`
    /// are signed counts. `sample` lists names and UUIDs in response order, with
    /// opted-out players anonymized in the initial sample. The server's setting
    /// to hide online players starts with an empty sample and still shows counts;
    /// set `hide_players` to omit the entire players section instead.
    ///
    /// Set `cancelled` or return `Flow::Cancel` to close the connection without
    /// a status response. The server settings, advertised version and secure-chat
    /// flag do not change. A response that cannot be encoded or exceeds the status
    /// packet's string limit closes only this connection, without truncation or
    /// a substitute response.
    #[hook(
        target = "sunrise_client::game::lan::Game<'_>::native_server_list_ping",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        read_only = ["protocol", "virtual_host"],
        neoforge = "none",
        paper = "PaperServerListPingEvent"
    )]
    fn server_list_ping(
        protocol: i32,
        virtual_host: &str,
        motd: &mut Text,
        favicon: &mut Option<Vec<u8>>,
        online: &mut i32,
        max_players: &mut i32,
        sample: &mut Vec<ListedPlayerInfo>,
        hide_players: &mut bool,
        cancel: &mut bool,
    );
}
