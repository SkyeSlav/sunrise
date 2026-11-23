//! A server mod's clientbound play packets (`PlayerRef::send_play_packets`): checked whole
//! before any goes out, then published as one ordered run.
use super::PlayerSender;
use mod_api::{ClientboundPlayPacket, SendClientboundPacketsError as Refusal};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::{Codec, Ident, Reader},
    ids::play::clientbound as ids,
    item::Names,
    packets::{
        custom_payload,
        play::{BUNDLE_SIZE_LIMIT, validate_clientbound_body},
    },
};

impl PlayerSender {
    /// Each packet's id and body on the wire, in order, between two bundle delimiters when
    /// `bundle` is asked for two or more. Every packet is checked first, against the
    /// connection's registry `names` and its payload channels; the first refusal stops it.
    pub fn prepare_mod_play_batch(
        &self,
        names: &dyn Names,
        packets: &[ClientboundPlayPacket],
        bundle: bool,
    ) -> Result<Vec<Vec<u8>>, Refusal> {
        let bundled = bundle && packets.len() >= 2;
        if bundled && packets.len() > BUNDLE_SIZE_LIMIT {
            return Err(Refusal::BundleTooLong);
        }
        let mut bodies = Vec::with_capacity(packets.len() + 2 * usize::from(bundled));
        let delimiter = || {
            let mut w = Writer::new();
            w.varint(ids::BUNDLE_DELIMITER);
            w.data
        };
        if bundled {
            bodies.push(delimiter());
        }
        for (index, packet) in packets.iter().enumerate() {
            let id = (packet.id.namespace() == "minecraft")
                .then(|| ids::NAMES.iter().position(|name| *name == packet.id.path()))
                .flatten()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| Refusal::UnknownPacket {
                    index,
                    id: packet.id.clone(),
                })?;
            if matches!(id, ids::BUNDLE_DELIMITER | ids::START_CONFIGURATION) {
                return Err(Refusal::ServerOnly { index });
            }
            if id == ids::CUSTOM_PAYLOAD {
                self.check_mod_payload(index, &packet.body)?;
            }
            validate_clientbound_body(id, &packet.body, names)
                .map_err(|_| Refusal::InvalidBody { index })?;
            let mut w = Writer::new();
            w.varint(id);
            w.bytes(&packet.body);
            bodies.push(w.data);
        }
        if bundled {
            bodies.push(delimiter());
        }
        Ok(bodies)
    }

    /// `ClientboundCustomPayloadPacket`: the channel is neither reserved nor this
    /// connection's payload transport's, and what follows it fits vanilla's clientbound limit.
    fn check_mod_payload(&self, index: usize, body: &[u8]) -> Result<(), Refusal> {
        let mut r = Reader::new(body);
        let Ok(channel) = <Ident as Codec<String>>::read(&mut r) else {
            return Err(Refusal::InvalidBody { index });
        };
        if custom_payload::is_reserved_channel(&channel)
            || self.payload_transport().local.owns(&channel)
        {
            return Err(Refusal::ProtectedPayload {
                index,
                channel: mod_api::PayloadId(channel),
            });
        }
        let size = r.remaining();
        if size > custom_payload::CLIENTBOUND_LIMIT {
            return Err(Refusal::PayloadTooLarge {
                index,
                size: size as u64,
                limit: custom_payload::CLIENTBOUND_LIMIT as u64,
            });
        }
        Ok(())
    }

    /// Publishes a prepared run in one piece, after anything already queued.
    pub fn send_mod_play_batch(&self, bodies: Vec<Vec<u8>>) {
        self.send_batch(bodies);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Identifier;
    use sunrise_protocol::item::NoNames;

    fn packet(name: &str, body: &[u8]) -> ClientboundPlayPacket {
        ClientboundPlayPacket::new(
            Identifier::parse(&format!("minecraft:{name}")).unwrap(),
            body.to_vec(),
        )
    }

    fn sender() -> PlayerSender {
        let (tx, rx) = std::sync::mpsc::channel();
        std::mem::forget(rx);
        PlayerSender::new(tx, Default::default())
    }

    #[test]
    fn a_run_is_the_packets_in_order_bundled_only_when_asked() {
        let s = sender();
        // set_camera (0x5f) with entity 300, set_chunk_cache_radius (0x61) with 8.
        let run = [
            packet("set_camera", &[0xac, 0x02]),
            packet("set_chunk_cache_radius", &[8]),
        ];
        let plain = vec![vec![0x5f, 0xac, 0x02], vec![0x61, 0x08]];
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &run, false).unwrap(),
            plain
        );
        let bundled = s.prepare_mod_play_batch(&NoNames, &run, true).unwrap();
        assert_eq!(
            bundled,
            [vec![0], plain[0].clone(), plain[1].clone(), vec![0]]
        );
        // One packet or none is never bundled.
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &run[..1], true).unwrap(),
            plain[..1]
        );
        assert!(
            s.prepare_mod_play_batch(&NoNames, &[], true)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn one_bad_packet_refuses_the_whole_run() {
        let s = sender();
        let good = packet("set_chunk_cache_radius", &[8]);
        let refused = |second: ClientboundPlayPacket| {
            s.prepare_mod_play_batch(&NoNames, &[good.clone(), second], false)
                .unwrap_err()
        };
        assert_eq!(
            refused(packet("set_chunk_cache_radius", &[])),
            Refusal::InvalidBody { index: 1 }
        );
        let foreign =
            ClientboundPlayPacket::new(Identifier::parse("fixture:set_camera").unwrap(), vec![0]);
        assert!(matches!(
            refused(foreign),
            Refusal::UnknownPacket { index: 1, .. }
        ));
        assert_eq!(
            refused(packet("bundle_delimiter", &[])),
            Refusal::ServerOnly { index: 1 }
        );
        assert_eq!(
            refused(packet("start_configuration", &[])),
            Refusal::ServerOnly { index: 1 }
        );
    }

    #[test]
    fn a_bundle_holds_at_most_vanillas_4096() {
        let s = sender();
        let packets = vec![packet("set_chunk_cache_radius", &[8]); BUNDLE_SIZE_LIMIT];
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &packets, true)
                .unwrap()
                .len(),
            BUNDLE_SIZE_LIMIT + 2
        );
        let packets = vec![packet("set_chunk_cache_radius", &[8]); BUNDLE_SIZE_LIMIT + 1];
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &packets, true)
                .unwrap_err(),
            Refusal::BundleTooLong
        );
        // Unbundled, the same list is only so many packets.
        assert!(s.prepare_mod_play_batch(&NoNames, &packets, false).is_ok());
    }

    #[test]
    fn custom_payloads_keep_to_their_channel_and_size() {
        let s = sender();
        let payload = |channel: &str, size: usize| {
            let mut w = Writer::new();
            w.string(channel);
            w.bytes(&vec![7; size]);
            packet("custom_payload", &w.data)
        };
        let limit = custom_payload::CLIENTBOUND_LIMIT;
        assert!(
            s.prepare_mod_play_batch(&NoNames, &[payload("fixture:data", limit)], false)
                .is_ok()
        );
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &[payload("fixture:data", limit + 1)], false)
                .unwrap_err(),
            Refusal::PayloadTooLarge {
                index: 0,
                size: limit as u64 + 1,
                limit: limit as u64
            }
        );
        let reserved = sunrise_protocol::payload_negotiation::CHANNEL;
        assert_eq!(
            s.prepare_mod_play_batch(&NoNames, &[payload(reserved, 1)], false)
                .unwrap_err(),
            Refusal::ProtectedPayload {
                index: 0,
                channel: mod_api::PayloadId(reserved.to_owned())
            }
        );
    }
}
