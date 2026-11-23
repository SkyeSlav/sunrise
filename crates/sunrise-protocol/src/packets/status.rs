//! Status state: server list ping.
use crate::{
    buf::{Long, StrN},
    packet, packet_enum,
    packets::Empty,
};

/// UTF-16 unit bound from `ClientboundStatusResponsePacket.STREAM_CODEC`.
pub const MAX_JSON_UTF16: usize = 32_767;
/// Encoded byte bound from `Utf8String.write`.
pub const MAX_JSON_BYTES: usize = MAX_JSON_UTF16 * 3;

packet!(StatusResponse { json: String => StrN<MAX_JSON_UTF16> });

impl StatusResponse {
    /// Encode the complete response packet, refusing strings outside vanilla's bounds.
    pub fn encode_checked(self) -> crate::Result<Vec<u8>> {
        if self.json.len() > MAX_JSON_BYTES || self.json.encode_utf16().count() > MAX_JSON_UTF16 {
            return Err(crate::Error::new(
                "status JSON exceeds the vanilla string limit",
            ));
        }
        Ok(Clientbound::StatusResponse(self).encode())
    }
}

packet!(Ping { time: i64 => Long });

packet_enum!(Clientbound, crate::ids::status::clientbound {
    StatusResponse(StatusResponse) = STATUS_RESPONSE,
    PongResponse(Ping) = PONG_RESPONSE,
});
packet_enum!(Serverbound, crate::ids::status::serverbound {
    StatusRequest(Empty) = STATUS_REQUEST,
    PingRequest(Ping) = PING_REQUEST,
});

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.

// End generated retained traversals.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::round_trip;

    #[test]
    fn outbound_status_counts_utf16_units_including_json_syntax() {
        let supplementary = "😀".repeat(16_382);
        let json = format!("\"{supplementary}a\"");
        let packet = StatusResponse { json: json.clone() }
            .encode_checked()
            .unwrap();
        let mut reader = crate::buf::Reader::new(&packet);
        let id = reader.varint().unwrap();
        let Clientbound::StatusResponse(decoded) = Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected a status response");
        };
        assert_eq!(decoded.json, json);

        let too_long = format!("\"{supplementary}aa\"");
        assert!(StatusResponse { json: too_long }.encode_checked().is_err());
    }

    #[test]
    fn status_round_trips() {
        round_trip(
            Clientbound::StatusResponse(StatusResponse {
                json: "{\"version\":{}}".into(),
            }),
            Clientbound::encode,
            Clientbound::decode,
        );
        round_trip(
            Clientbound::PongResponse(Ping { time: 123 }),
            Clientbound::encode,
            Clientbound::decode,
        );
        round_trip(
            Serverbound::StatusRequest(Empty),
            Serverbound::encode,
            Serverbound::decode,
        );
        round_trip(
            Serverbound::PingRequest(Ping { time: -5 }),
            Serverbound::encode,
            Serverbound::decode,
        );
    }
}
