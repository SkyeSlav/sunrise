//! Named timing scopes for native mods on the client and server.
//!
//! Zones appear in the engine's timeline capture, not in `/modprofile`'s aggregate call timings.

use std::marker::PhantomData;

/// A timing scope that closes when dropped, including on early return.
///
/// Keep the guard on the thread that opened it and drop nested guards in reverse order.
/// A scope that outlives its capture is discarded.
#[must_use = "bind the profiling zone to keep it open until the end of the scope"]
pub struct Zone {
    finish: Option<Box<dyn FnOnce()>>,
    _thread: PhantomData<*const ()>,
}

impl Drop for Zone {
    fn drop(&mut self) {
        if let Some(finish) = self.finish.take() {
            finish();
        }
    }
}

/// Time the current thread's work under `label`, inside any enclosing engine or mod zone.
///
/// The label may be chosen at runtime and need not outlive the call. Active captures save it
/// verbatim, so don't include private data. With no active timeline (including in standalone
/// mod tests), this returns an inert guard without allocating. It does not start a capture.
#[inline]
pub fn zone(label: &str) -> Zone {
    Zone {
        finish: recorder::get().and_then(|record| record(label)),
        _thread: PhantomData,
    }
}

pub(crate) mod recorder {
    use std::sync::OnceLock;

    type Recorder = fn(&str) -> Option<Box<dyn FnOnce()>>;

    static RECORDER: OnceLock<Recorder> = OnceLock::new();

    /// Connect the engine's timeline recorder before native mod registration.
    pub fn install_named_zone_recorder(record: Recorder) -> Result<(), &'static str> {
        RECORDER
            .set(record)
            .map_err(|_| "the named zone recorder is already installed")
    }

    #[cfg(any(test, feature = "testing"))]
    thread_local! {
        static TEST_RECORDER: std::cell::Cell<Option<Recorder>> = const { std::cell::Cell::new(None) };
    }

    #[inline]
    pub(super) fn get() -> Option<Recorder> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(record) = TEST_RECORDER.try_with(std::cell::Cell::get).ok().flatten() {
            return Some(record);
        }
        RECORDER.get().copied()
    }

    /// Run a test with a named-zone recorder installed on its thread only.
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    pub fn with_named_zone_recorder<R>(record: Recorder, f: impl FnOnce() -> R) -> R {
        struct Restore(Option<Recorder>);
        impl Drop for Restore {
            fn drop(&mut self) {
                TEST_RECORDER.set(self.0);
            }
        }
        let _restore = Restore(TEST_RECORDER.replace(Some(record)));
        f()
    }
}
