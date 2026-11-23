//! One play connection's packet totals for the mods' server diagnostics: each play packet's
//! VarInt id and body, before compression and framing, as `PacketEncoder.encode` writes and
//! `PacketDecoder.decode` reads them.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use sunrise_protocol::ids::play::{clientbound, serverbound};

/// Which way a play packet went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Clientbound,
    Serverbound,
}

/// One packet type's totals since the last take.
#[derive(Debug, PartialEq, Eq)]
pub struct PacketTotal {
    pub direction: Direction,
    /// The packet type's name in its direction (`keep_alive`).
    pub name: &'static str,
    pub packets: u64,
    pub bytes: u64,
}

/// A connection's totals by packet id. Ids outside the play registries are never counted.
pub struct PacketCounts {
    /// The server's switch, shared by every connection; none for a connection outside a server.
    on: OnceLock<Arc<AtomicBool>>,
    /// Packets and bytes for each id, clientbound then serverbound.
    rows: Mutex<[Box<[[u64; 2]]>; 2]>,
}

impl Default for PacketCounts {
    fn default() -> Self {
        Self {
            on: OnceLock::new(),
            rows: Mutex::new([
                vec![[0; 2]; clientbound::NAMES.len()].into(),
                vec![[0; 2]; serverbound::NAMES.len()].into(),
            ]),
        }
    }
}

impl PacketCounts {
    /// Count while the server's `on` switch is set.
    pub(super) fn switch_with(&self, on: Arc<AtomicBool>) {
        let _ = self.on.set(on);
    }

    /// One packet of `bytes` (its VarInt id and payload) with play id `id`.
    pub(super) fn add(&self, direction: Direction, id: i32, bytes: usize) {
        if !self.on.get().is_some_and(|on| on.load(Ordering::Relaxed)) {
            return;
        }
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(row) = usize::try_from(id)
            .ok()
            .and_then(|id| rows[direction as usize].get_mut(id))
        {
            row[0] += 1;
            row[1] += bytes as u64;
        }
    }

    /// The totals since the last take, and clears them: clientbound first, then by name.
    pub fn take(&self) -> Vec<PacketTotal> {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut totals = Vec::new();
        for (direction, names) in [
            (Direction::Clientbound, clientbound::NAMES),
            (Direction::Serverbound, serverbound::NAMES),
        ] {
            let first = totals.len();
            for (row, name) in rows[direction as usize].iter_mut().zip(names) {
                if row[0] > 0 {
                    let [packets, bytes] = std::mem::take(row);
                    totals.push(PacketTotal {
                        direction,
                        name,
                        packets,
                        bytes,
                    });
                }
            }
            totals[first..].sort_unstable_by_key(|total| total.name);
        }
        totals
    }
}

/// A prepared body's packet id and its length before compression, which its framed bytes no
/// longer show.
#[derive(Clone, Copy, Debug)]
pub(super) struct Origin {
    pub(super) id: i32,
    pub(super) len: usize,
}

impl Origin {
    /// The origin of a whole packet body (its VarInt id and payload); a body with no id counts
    /// as no packet type.
    pub(super) fn of(body: &[u8]) -> Self {
        Self {
            id: sunrise_protocol::buf::Reader::new(body)
                .varint()
                .unwrap_or(-1),
            len: body.len(),
        }
    }
}

/// The length of `id` as a VarInt: the start of a packet body.
pub(super) fn varint_len(id: i32) -> usize {
    (id as u32 | 1).ilog2() as usize / 7 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_count_only_while_on_and_keep_what_no_one_took() {
        let counts = PacketCounts::default();
        let on = Arc::new(AtomicBool::new(false));
        counts.switch_with(on.clone());
        counts.add(Direction::Clientbound, clientbound::KEEP_ALIVE, 9);
        assert!(counts.take().is_empty());

        on.store(true, Ordering::Relaxed);
        counts.add(Direction::Serverbound, serverbound::KEEP_ALIVE, 9);
        counts.add(Direction::Clientbound, clientbound::KEEP_ALIVE, 9);
        counts.add(Direction::Clientbound, clientbound::BUNDLE_DELIMITER, 1);
        counts.add(Direction::Clientbound, clientbound::BUNDLE_DELIMITER, 1);
        // An id past the play registry adds no row.
        counts.add(Direction::Clientbound, clientbound::NAMES.len() as i32, 5);
        on.store(false, Ordering::Relaxed);
        counts.add(Direction::Clientbound, clientbound::KEEP_ALIVE, 9);
        let total = |direction, name, packets, bytes| PacketTotal {
            direction,
            name,
            packets,
            bytes,
        };
        assert_eq!(
            counts.take(),
            [
                total(Direction::Clientbound, "bundle_delimiter", 2, 2),
                total(Direction::Clientbound, "keep_alive", 1, 9),
                total(Direction::Serverbound, "keep_alive", 1, 9),
            ]
        );
        assert!(counts.take().is_empty());
    }
}
