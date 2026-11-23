//! Body checks for the debug value packets (`ClientboundDebugBlockValuePacket`,
//! `ClientboundDebugChunkValuePacket`, `ClientboundDebugEntityValuePacket`,
//! `ClientboundDebugEventPacket`): `DebugSubscription.Update` and `DebugSubscription.Event`
//! dispatch on a `debug_subscription` registry id to that subscription's value codec
//! (`DebugSubscriptions`). Each reader consumes exactly what the vanilla codec reads and
//! fails where it throws; it keeps no value.
use crate::{Error, Result, buf::Reader};

/// `ByteBufCodecs.list()` and `collection(..)` without a bound read at most this many.
const UNBOUNDED: usize = i32::MAX as usize;

/// `ClientboundDebugBlockValuePacket`: a `BlockPos`, then a `DebugSubscription.Update`.
pub(super) fn block_value(r: &mut Reader) -> Result<()> {
    r.i64()?;
    update(r)
}
/// `ClientboundDebugChunkValuePacket`: a `ChunkPos` (one long), then an update.
pub(super) fn chunk_value(r: &mut Reader) -> Result<()> {
    r.i64()?;
    update(r)
}
/// `ClientboundDebugEntityValuePacket`: the entity id (VarInt), then an update.
pub(super) fn entity_value(r: &mut Reader) -> Result<()> {
    r.varint()?;
    update(r)
}
/// `ClientboundDebugEventPacket`: a `DebugSubscription.Event`, a subscription and its value.
pub(super) fn event(r: &mut Reader) -> Result<()> {
    let subscription = subscription(r)?;
    value(r, &subscription)
}

/// `DebugSubscription.Update`: a subscription, then its value in `ByteBufCodecs.optional`.
fn update(r: &mut Reader) -> Result<()> {
    let subscription = subscription(r)?;
    if r.bool()? {
        value(r, &subscription)?;
    }
    Ok(())
}

/// `ByteBufCodecs.registry(Registries.DEBUG_SUBSCRIPTION)`: a registered id's name.
fn subscription(r: &mut Reader) -> Result<String> {
    let id = r.varint()?;
    r.registry_name("minecraft:debug_subscription", id)
}

/// The subscription's `valueStreamCodec`; one without a value codec cannot be sent.
fn value(r: &mut Reader, subscription: &str) -> Result<()> {
    match subscription {
        // DebugBeeInfo.
        "minecraft:bees" => {
            optional(r, block_pos)?;
            optional(r, block_pos)?;
            r.varint()?;
            list(r, block_pos)
        }
        // DebugBrainDump.
        "minecraft:brains" => {
            string(r)?;
            string(r)?;
            r.i32()?;
            r.f32()?;
            r.f32()?;
            string(r)?;
            r.bool()?;
            r.i32()?;
            for _ in 0..4 {
                list(r, string)?;
            }
            list(r, block_pos)?;
            list(r, block_pos)
        }
        // DebugBreezeInfo.
        "minecraft:breezes" => {
            optional(r, |r| r.varint().map(drop))?;
            optional(r, block_pos)
        }
        // DebugGoalInfo: priority, running, `stringUtf8(255)` name.
        "minecraft:goal_selectors" => list(r, |r| {
            r.varint()?;
            r.bool()?;
            r.string(255).map(drop)
        }),
        // DebugPathInfo: `Path.DEBUG_STREAM_CODEC`, then the max node distance.
        "minecraft:entity_paths" => {
            r.bool()?;
            r.i32()?;
            block_pos(r)?;
            list(r, node)?;
            // Path.DebugData: open set, closed set, targets.
            list(r, node)?;
            list(r, node)?;
            list(r, node)?;
            r.f32().map(drop)
        }
        // DebugEntityBlockIntersection: `idMapper` over `ByIdMap` with `ZERO` for any id.
        "minecraft:entity_block_intersections" => r.varint().map(drop),
        // DebugHiveInfo: a registered block, occupants, honey level, sedated.
        "minecraft:bee_hives" => {
            registered(r, "minecraft:block")?;
            r.varint()?;
            r.varint()?;
            r.bool().map(drop)
        }
        // DebugPoiInfo: position, a registered POI type, free tickets.
        "minecraft:pois" => {
            block_pos(r)?;
            registered(r, "minecraft:point_of_interest_type")?;
            r.varint().map(drop)
        }
        // Orientation.STREAM_CODEC: `fromIndex` reads one of its 48 orientations.
        "minecraft:redstone_wire_orientations" => match r.varint()? {
            0..48 => Ok(()),
            index => Err(Error::new(format!("invalid wire orientation {index}"))),
        },
        // Unit.STREAM_CODEC writes nothing.
        "minecraft:village_sections" => Ok(()),
        "minecraft:raids" => list(r, block_pos),
        // DebugStructureInfo: a bounding box and its pieces (a box and whether it starts).
        "minecraft:structures" => list(r, |r| {
            bounding_box(r)?;
            list(r, |r| {
                bounding_box(r)?;
                r.bool().map(drop)
            })
        }),
        // DebugGameEventListenerInfo: the listener radius.
        "minecraft:game_event_listeners" => r.varint().map(drop),
        "minecraft:neighbor_updates" => block_pos(r),
        // DebugGameEventInfo: a registered game event and a `Vec3`.
        "minecraft:game_events" => {
            registered(r, "minecraft:game_event")?;
            for _ in 0..3 {
                r.f64()?;
            }
            Ok(())
        }
        other => Err(Error::new(format!(
            "debug subscription {other} has no value"
        ))),
    }
}

/// `BlockPos.STREAM_CODEC`: one packed long.
fn block_pos(r: &mut Reader) -> Result<()> {
    r.i64().map(drop)
}
/// `BoundingBox.STREAM_CODEC`: its two corners.
fn bounding_box(r: &mut Reader) -> Result<()> {
    block_pos(r)?;
    block_pos(r)
}
/// `ByteBufCodecs.STRING_UTF8`.
fn string(r: &mut Reader) -> Result<()> {
    r.string(32767).map(drop)
}
/// `Node.DEBUG_STREAM_CODEC` (and `Target`'s, the same fields): x, y, z, walked distance,
/// cost malus, closed, a `PathType` (`ByIdMap` with `ZERO` for any id), f.
fn node(r: &mut Reader) -> Result<()> {
    for _ in 0..3 {
        r.i32()?;
    }
    r.f32()?;
    r.f32()?;
    r.bool()?;
    r.varint()?;
    r.f32().map(drop)
}
/// `ByteBufCodecs.holderRegistry` and `registry`: an id the registry has.
fn registered(r: &mut Reader, registry: &str) -> Result<()> {
    let id = r.varint()?;
    r.registry_name(registry, id).map(drop)
}
/// `ByteBufCodecs.optional`: a presence flag, then the value.
fn optional(r: &mut Reader, value: impl FnOnce(&mut Reader) -> Result<()>) -> Result<()> {
    if r.bool()? { value(r) } else { Ok(()) }
}
/// `ByteBufCodecs.list()` and an unbounded `collection`: a count, then the elements.
fn list(r: &mut Reader, mut element: impl FnMut(&mut Reader) -> Result<()>) -> Result<()> {
    for _ in 0..r.count(UNBOUNDED)? {
        element(r)?;
    }
    Ok(())
}
