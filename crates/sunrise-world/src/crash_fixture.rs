//! Opt-in crash points for the dedicated-server harnesses. Ordinary saves never wait here.
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
use sunrise_core::position::ChunkPos;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CrashFixture {
    Panic(&'static str),
    #[cfg(target_os = "linux")]
    SaveLevel,
    #[cfg(target_os = "linux")]
    SavePlayer,
    #[cfg(target_os = "linux")]
    SaveRegion(ChunkPos),
}

pub fn selected() -> Option<CrashFixture> {
    static FIXTURE: OnceLock<Option<CrashFixture>> = OnceLock::new();
    *FIXTURE.get_or_init(
        || match std::env::var("SUNRISE_CRASH_FIXTURE").ok()?.as_str() {
            "guest" => Some(CrashFixture::Panic("guest")),
            "hub" => Some(CrashFixture::Panic("hub")),
            #[cfg(target_os = "linux")]
            "save-level" => Some(CrashFixture::SaveLevel),
            #[cfg(target_os = "linux")]
            "save-player" => Some(CrashFixture::SavePlayer),
            #[cfg(target_os = "linux")]
            point => {
                let (x, z) = point.strip_prefix("save-region:")?.split_once(',')?;
                Some(CrashFixture::SaveRegion(ChunkPos::new(
                    x.parse().ok()?,
                    z.parse().ok()?,
                )))
            }
            #[cfg(not(target_os = "linux"))]
            _ => None,
        },
    )
}

/// The harness arms this point after startup, just before `save-all`.
#[cfg(target_os = "linux")]
pub fn await_save_kill(point: &str) {
    if std::path::Path::new(".sunrise-save-fault-armed").is_file() {
        eprintln!("save crash fixture reached {point}");
        loop {
            std::thread::park();
        }
    }
}
