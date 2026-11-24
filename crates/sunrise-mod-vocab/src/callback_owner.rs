//! Owner enablement, attribution and panic containment for retained resources.
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn call<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, String> {
    if !crate::mod_index_on(index) || poisoned.load(Ordering::Acquire) {
        return Err(format!("{kind} owner is disabled"));
    }
    let _guard = crate::mark_mod(owner, operation);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(invoke)).map_err(|_| {
        poisoned.store(true, Ordering::Release);
        format!("{kind} callback panicked; it is disabled")
    })
}
