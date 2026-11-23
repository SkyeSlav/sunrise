pub use item::transaction::{
    HostTransaction, RootCommitEffect, ScratchError, ScratchScopeError, TransactionContext,
    TransactionError, TransactionLifecycle, TransactionScopeError, TransactionUnavailable,
};
mod __private {
    pub use crate::item::transaction::PendingRootCommitEffect;
    /// A journal frame's process-local identity, built and matched by the game's journal.
    pub use crate::item::transaction::TransactionFrame;
}
