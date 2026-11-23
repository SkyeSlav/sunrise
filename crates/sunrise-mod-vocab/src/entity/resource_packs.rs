//! Why a player's resource pack push or pop sent nothing.

/// Why `player.push_resource_pack` or `player.pop_resource_pack` answered with nothing sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ResourcePackRefusal {
    /// The call was made on a client; packs are offered by the server.
    Client,
    /// The handle no longer identifies a player in the server's list: it is stale, or the
    /// player has left.
    Stale,
    /// The player has no connection to send to (a fake player).
    NoConnection,
    /// The URL is longer than the packet carries: 32767 UTF-16 code units
    /// (`ClientboundResourcePackPushPacket`'s `ByteBufCodecs.STRING_UTF8`).
    UrlTooLong,
    /// The hash is longer than the packet carries: 40 UTF-16 code units
    /// (`ClientboundResourcePackPushPacket`'s `ByteBufCodecs.stringUtf8(40)`).
    HashTooLong,
    /// The prompt is text the packet refuses: deeper than 512 component nodes.
    BadPrompt,
}

impl crate::NoBackend for ResourcePackRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}

impl From<crate::TextFormError> for ResourcePackRefusal {
    fn from(_: crate::TextFormError) -> Self {
        Self::BadPrompt
    }
}
