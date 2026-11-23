//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// Keep a held-out entity inaccessible until its copy has been stored back, also when the
    /// action unwinds.
    pub(in crate::game) fn with_entity_held_out<R>(
        &mut self,
        id: u32,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        if mod_api::__private::mods().is_empty() {
            return action(self);
        }
        let depth = self.held_out.len();
        self.held_out.push(id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self)));
        self.held_out.truncate(depth);
        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }

    /// Let a shared callback read the current stored row, without letting a later
    /// mutable callback touch the entity before its tick finishes storing it back. The hold
    /// is back in place when the callback unwinds.
    pub(in crate::game) fn with_held_entity_read<R>(
        &mut self,
        id: u32,
        action: impl FnOnce(&Self) -> R,
    ) -> R {
        let held = self.held_out.last() == Some(&id);
        if held {
            self.held_out.pop();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self)));
        if held {
            self.held_out.push(id);
        }
        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }
}
