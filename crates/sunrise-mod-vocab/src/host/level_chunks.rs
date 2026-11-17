//! The `level_chunks` area: chunks of an open level: handles, status, loading and resending. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_chunks` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelChunksBackend {
    /// Side: server.
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    #[access(WorldOnly)]
    fn level_chunk_ref(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
    /// Side: server.
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    #[access(WorldOnly)]
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool;
    /// Side: server.
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    #[access(WorldOnly)]
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus>;
    /// Side: server.
    /// Whether the chunk passes vanilla's seeded slime-chunk test, independent of
    /// dimension, height, biome and other spawn conditions. Works for unloaded chunks
    /// in active or parked open levels. None for a stale level, a client or a chunk
    /// outside world bounds. Does not load chunks or advance the world's random stream.
    #[access(WorldOnly)]
    fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool>;
    /// Side: server.
    /// `ChunkMap.getPlayers(pos, false)`: the players watching this chunk, those
    /// it is inside the view of and was sent to, in the order the server keeps
    /// its players. A player watches it from its `chunk_watch` until it leaves
    /// the view; Sunrise's far-ring columns past the view distance are not
    /// watched. Fake players watch nothing. Loaded or not, a chunk nobody
    /// watches gives an empty list. None on a client and for a stale handle.
    #[access(WorldOnly)]
    fn chunk_watchers(&self, chunk: ChunkRef) -> Option<Vec<PlayerRef>>;
    /// Side: both.
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    #[access(WorldOnly)]
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: both.
    /// Every loaded chunk of an open server level as handles, ordered by chunk x then z. None for
    /// a stale handle or on a client. Loads nothing.
    #[access(WorldOnly)]
    fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>>;
}
/// The handles' writes of the `level_chunks` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelChunksBackendMut {
    /// Side: server.
    /// Sends the loaded chunk whole again, with its light, block entities and biomes as they are
    /// at the end of the tick, to every player watching it (`chunk_watchers`), as a chunk
    /// `edit_blocks` changed is sent: one packet instead of a block update per changed block.
    /// Ok with nothing sent when no player is in the level. Each refusal queues nothing and is
    /// named by `ChunkRefusal`: `Stale`, `Client`, `NotInPlace` and `NotLoaded`.
    #[access(WorldOnly)]
    fn chunk_resend(&mut self, chunk: ChunkRef) -> Result<(), ChunkRefusal>;
}
