//! Moving blocks, their block entities and scheduled block ticks between levels (#5577).
use crate::{LevelRef, StructureMirror, StructureRotation, TransactionError};
use std::fmt;

/// `StructurePlaceSettings`' rotation, mirror and pivot for [`LevelRef::move_blocks`].
///
/// Mirroring runs first, then rotation about `pivot` (in coordinates local to the source
/// area's minimum corner, matching `StructureTemplate.transform`), then displacement by the
/// move's `offset`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BlockTransform {
    /// Rotation applied after mirroring (`StructurePlaceSettings.rotation`).
    pub rotation: StructureRotation,
    /// Mirror applied before rotating (`StructurePlaceSettings.mirror`).
    pub mirror: StructureMirror,
    /// Pivot about which the rotation turns (`StructurePlaceSettings.rotationPivot`).
    pub pivot: [i32; 3],
}

impl BlockTransform {
    /// Construct a transform with `rotation`, `mirror` and `pivot`.
    pub fn new(rotation: StructureRotation, mirror: StructureMirror, pivot: [i32; 3]) -> Self {
        Self {
            rotation,
            mirror,
            pivot,
        }
    }

    /// Return a copy of this transform with `rotation`.
    pub fn with_rotation(mut self, rotation: StructureRotation) -> Self {
        self.rotation = rotation;
        self
    }

    /// Return a copy of this transform with `mirror`.
    pub fn with_mirror(mut self, mirror: StructureMirror) -> Self {
        self.mirror = mirror;
        self
    }

    /// Return a copy of this transform with `pivot`.
    pub fn with_pivot(mut self, pivot: [i32; 3]) -> Self {
        self.pivot = pivot;
        self
    }
}

/// Counts reported by a successful [`LevelRef::move_blocks`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MovedBlocks {
    /// Destination cells whose block state changed (`Level.setBlock` returned `true`).
    pub blocks: u64,
    /// Block entities moved as the same holder from source to destination.
    pub block_entities: u64,
}

impl MovedBlocks {
    /// Construct a move summary from its changed-block and moved-block-entity counts.
    pub fn new(blocks: u64, block_entities: u64) -> Self {
        Self {
            blocks,
            block_entities,
        }
    }
}

// no wire: wraps local TransactionError and is returned only by in-process LevelRef::move_blocks.
/// Why [`LevelRef::move_blocks`] refused without changing either level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MoveBlocksError {
    /// A source or destination chunk is not loaded (`LevelReader.hasChunksAt`).
    NotLoaded {
        /// The level missing a loaded chunk in the source or destination box.
        level: LevelRef,
    },
    /// A block entity that cannot be taken out or put back, or whose transform listener refused.
    BlockEntityRefused {
        /// Position of the refused block entity.
        at: [i32; 3],
    },
    /// The transaction's own conflict or availability refusal: a handle went stale, the server
    /// is unavailable, or a conflicting write blocked the move.
    Transaction(TransactionError<()>),
}

impl crate::NoBackend for MoveBlocksError {
    fn no_backend() -> Self {
        Self::Transaction(TransactionError::Unavailable)
    }
}

impl fmt::Display for MoveBlocksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoaded { .. } => f.write_str("a source or destination chunk is not loaded"),
            Self::BlockEntityRefused { at } => {
                write!(f, "block entity at {at:?} refused to move")
            }
            Self::Transaction(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for MoveBlocksError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transaction(err) => Some(err),
            Self::NotLoaded { .. } | Self::BlockEntityRefused { .. } => None,
        }
    }
}
