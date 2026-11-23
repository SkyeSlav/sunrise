//! Modern status snapshots and their connection-to-server callback handoff.
use crate::server::json::write_quoted;
use mod_api::{__private::ServerListPingCall, ListedPlayerInfo, Text};
use serde_json::Value;
use std::{fmt, fmt::Write, sync::Arc, sync::mpsc::Sender};
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_protocol::{
    GAME_VERSION, PROTOCOL_VERSION,
    crypto::uuid_string,
    packets::status::{MAX_JSON_BYTES, StatusResponse},
};

#[derive(Clone)]
pub(super) struct Snapshot {
    motd: Text,
    favicon: Option<Vec<u8>>,
    online: i32,
    max_players: i32,
    sample: Vec<ListedPlayerInfo>,
    hide_players: bool,
    // The dedicated server's policy is fixed. Let the connection thread resolve its
    // startup key fetch on first use, just as it did before snapshots were published.
    secure_chat: Option<Arc<super::SecureChat>>,
}

impl Snapshot {
    pub(super) fn new(
        motd: String,
        favicon: Option<Vec<u8>>,
        max_players: usize,
        secure_chat: Option<Arc<super::SecureChat>>,
    ) -> Self {
        Self {
            motd: Text::literal(motd),
            favicon,
            online: 0,
            max_players: i32::try_from(max_players).unwrap_or(i32::MAX),
            sample: Vec::new(),
            hide_players: false,
            secure_chat,
        }
    }

    pub(super) fn set_players<'a>(
        &mut self,
        players: impl Iterator<Item = &'a super::guest::Guest>,
        hide_online_players: bool,
        random: &mut LegacyRandom,
    ) {
        if hide_online_players {
            self.online = i32::try_from(players.count()).unwrap_or(i32::MAX);
            return;
        }
        // MinecraftServer.buildPlayerStatus: choose a contiguous window in join order, then
        // shuffle it. Mth.nextInt makes no draw when the window covers the whole list.
        let players: Vec<_> = players.collect();
        self.online = i32::try_from(players.len()).unwrap_or(i32::MAX);
        let size = self.online.min(12);
        let start = if self.online == size {
            0
        } else {
            random.next_int_bound(self.online - size + 1).unwrap() as usize
        };
        self.sample = players[start..start + size as usize]
            .iter()
            .map(|player| {
                if player.info.allows_listing {
                    ListedPlayerInfo::new(player.name.clone(), player.uuid)
                } else {
                    ListedPlayerInfo::new("Anonymous Player", 0)
                }
            })
            .collect();
        for i in (1..size).rev() {
            let j = random.next_int_bound(i + 1).unwrap();
            self.sample.swap(i as usize, j as usize);
        }
    }

    pub(super) fn json(&self) -> Result<String, String> {
        let description =
            crate::api_text::to_component_json(&self.motd).map_err(|e| e.to_string())?;
        let mut json = Json(String::new());
        let result = json.write_status(self, &description);
        crate::component::discard_component(description);
        result.map_err(|_| "status JSON exceeds the vanilla string limit".to_owned())?;
        Ok(json.0)
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, String> {
        StatusResponse { json: self.json()? }
            .encode_checked()
            .map_err(|e| e.to_string())
    }
}

/// The request owns its handshake metadata. Dropping the pending request on server shutdown
/// releases the connection thread without writing a response.
pub struct Request {
    pub(super) snapshot: Arc<Snapshot>,
    pub(super) protocol: i32,
    pub(super) virtual_host: String,
    pub(super) reply: Sender<Option<Arc<Snapshot>>>,
}

impl Request {
    /// Lend the handler a private copy. No edits are made to the published snapshot.
    pub fn respond(self, edit: impl FnOnce(ServerListPingCall<'_>)) {
        let mut response = Arc::unwrap_or_clone(self.snapshot);
        let mut cancel = false;
        edit(ServerListPingCall {
            protocol: self.protocol,
            virtual_host: &self.virtual_host,
            motd: &mut response.motd,
            favicon: &mut response.favicon,
            online: &mut response.online,
            max_players: &mut response.max_players,
            sample: &mut response.sample,
            hide_players: &mut response.hide_players,
            cancel: &mut cancel,
        });
        let _ = self.reply.send((!cancel).then(|| Arc::new(response)));
    }
}

/// `MinecraftServer.loadStatusIcon` uses `PngInfo.fromBytes`, not a pixel decode.
/// At 64 × 64 the 16-bit buffer-size check in `PngInfo.fromStream` always passes.
pub fn valid_favicon(png: &[u8]) -> bool {
    const HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x40\0\0\0\x40";
    matches!(
        png.strip_prefix(HEADER),
        Some([1 | 2 | 4 | 8 | 16, 0 | 2 | 3 | 4 | 6, ..])
    )
}

// Stop serializing as soon as the packet's byte bound is reached. The packet encoder
// also checks the final UTF-16 length (a byte count is not a character count).
struct Json(String);

impl Write for Json {
    fn write_str(&mut self, fragment: &str) -> fmt::Result {
        if fragment.len() > MAX_JSON_BYTES - self.0.len() {
            return Err(fmt::Error);
        }
        self.0.push_str(fragment);
        Ok(())
    }
}

impl Json {
    fn write_status(&mut self, status: &Snapshot, description: &Value) -> fmt::Result {
        self.write_str("{")?;
        // ComponentSerialization: collapse an unstyled, childless literal to a string.
        let literal = description
            .as_object()
            .filter(|fields| fields.len() == 1)
            .and_then(|fields| fields.get("text"))
            .and_then(Value::as_str);
        match literal {
            Some("") => {}
            Some(text) => {
                self.write_str(r#""description":"#)?;
                write_quoted(self, text, false)?;
                self.write_str(",")?;
            }
            None => {
                self.write_str(r#""description":"#)?;
                self.write_component(description)?;
                self.write_str(",")?;
            }
        }
        if !status.hide_players {
            write!(
                self,
                "\"players\":{{\"max\":{},\"online\":{}",
                status.max_players, status.online
            )?;
            if !status.sample.is_empty() {
                self.write_str(r#","sample":["#)?;
                for (i, player) in status.sample.iter().enumerate() {
                    if i != 0 {
                        self.write_str(",")?;
                    }
                    write!(self, "{{\"id\":\"{}\",\"name\":", uuid_string(player.uuid))?;
                    write_quoted(self, &player.name, false)?;
                    self.write_str("}")?;
                }
                self.write_str("]")?;
            }
            self.write_str("},")?;
        }
        self.write_str(r#""version":{"name":"#)?;
        write_quoted(self, GAME_VERSION, false)?;
        write!(self, ",\"protocol\":{PROTOCOL_VERSION}}}")?;
        if let Some(icon) = status.favicon.as_deref().filter(|icon| valid_favicon(icon)) {
            const PREFIX: &str = ",\"favicon\":\"data:image/png;base64,";
            // Size the encoded field before allocating its base64 representation.
            let remaining = MAX_JSON_BYTES - self.0.len();
            if icon.len().div_ceil(3) > remaining.saturating_sub(PREFIX.len() + 2) / 4 {
                return Err(fmt::Error);
            }
            self.write_str(PREFIX)?;
            self.write_str(&super::base64(icon))?;
            self.write_str("\"")?;
        }
        if status
            .secure_chat
            .as_ref()
            .is_some_and(|secure_chat| secure_chat.enforced())
        {
            self.write_str(r#","enforcesSecureChat":true"#)?;
        }
        self.write_str("}")
    }

    /// Traverse mod-provided component JSON iteratively, keeping only the active iterators.
    fn write_component(&mut self, root: &Value) -> fmt::Result {
        enum Step<'a> {
            Value(&'a Value),
            Array(std::slice::Iter<'a, Value>, bool),
            Object(serde_json::map::Iter<'a>, bool),
        }
        let mut stack = vec![Step::Value(root)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Value(Value::Array(values)) => {
                    self.write_str("[")?;
                    stack.push(Step::Array(values.iter(), true));
                }
                Step::Value(Value::Object(fields)) => {
                    self.write_str("{")?;
                    stack.push(Step::Object(fields.iter(), true));
                }
                Step::Value(Value::String(text)) => write_quoted(self, text, false)?,
                Step::Value(Value::Number(number)) => write!(self, "{number}")?,
                Step::Value(Value::Bool(value)) => write!(self, "{value}")?,
                Step::Value(Value::Null) => self.write_str("null")?,
                Step::Array(mut values, first) => {
                    if let Some(value) = values.next() {
                        if !first {
                            self.write_str(",")?;
                        }
                        stack.push(Step::Array(values, false));
                        stack.push(Step::Value(value));
                    } else {
                        self.write_str("]")?;
                    }
                }
                Step::Object(mut fields, first) => {
                    if let Some((key, value)) = fields.find(|(_, value)| !value.is_null()) {
                        if !first {
                            self.write_str(",")?;
                        }
                        write_quoted(self, key, false)?;
                        self.write_str(":")?;
                        stack.push(Step::Object(fields, false));
                        stack.push(Step::Value(value));
                    } else {
                        self.write_str("}")?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_omits_invalid_icons_and_preserves_valid_png_bytes() {
        let png = |size| {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, size, size);
            encoder.set_color(png::ColorType::Rgba);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![0; (size * size * 4) as usize])
                .unwrap();
            out
        };
        let with_icon = |icon| {
            let snapshot = Snapshot::new(String::new(), Some(icon), 8, None);
            serde_json::from_str::<Value>(&snapshot.json().unwrap()).unwrap()
        };
        let valid = png(64);
        let reply = with_icon(valid.clone());
        assert_eq!(
            reply["favicon"],
            format!("data:image/png;base64,{}", super::super::base64(&valid))
        );
        assert_eq!(reply.get("description"), None);
        assert_eq!(with_icon(png(32)).get("favicon"), None);
        assert_eq!(with_icon(valid[..24].to_vec()).get("favicon"), None);
        let mut wrong_chunk_length = valid;
        wrong_chunk_length[11] = 12;
        assert_eq!(with_icon(wrong_chunk_length).get("favicon"), None);
    }

    #[test]
    fn component_serialization_preserves_nested_text_and_gson_escapes() {
        let mut snapshot = Snapshot::new(String::new(), None, 8, None);
        let mut leaf = Text::literal("line\u{2028}paragraph\u{2029}");
        for _ in 0..600 {
            let mut parent = Text::literal("parent");
            parent.children.push(leaf);
            leaf = parent;
        }
        snapshot.motd = leaf;
        let packet = snapshot.encode().unwrap();
        let mut reader = sunrise_protocol::buf::Reader::new(&packet);
        let id = reader.varint().unwrap();
        let sunrise_protocol::packets::status::Clientbound::StatusResponse(response) =
            sunrise_protocol::packets::status::Clientbound::decode(id, reader.rest()).unwrap()
        else {
            panic!("expected a status response");
        };
        assert_eq!(response.json.matches(r#""text":"parent""#).count(), 600);
        assert!(
            response
                .json
                .contains(r#""text":"line\u2028paragraph\u2029""#)
        );
    }
}
