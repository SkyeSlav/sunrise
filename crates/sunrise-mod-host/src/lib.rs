//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, RUSTC_VERSION, main};

mod loading;
pub use loading::main_with;

/// Settles the guest stack policy from the mod engine, then establishes main-thread stack
/// capacity, before compiler-child handling. A refusal here is the same exit as a stack that
/// cannot be made large enough.
pub fn prepare_main_stack() {
    if let Err(error) = sunrise_mod_wasm::install_stack_policy() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    sunrise_guest_stack::prepare_main();
}
pub use sunrise_mod_wasm::compile_child;

/// Handles the private compiler invocation before any game startup work.
pub fn exit_if_compile_child() {
    prepare_main_stack();
    if let Some(code) = compile_child(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }
}

/// A typed API-point listener supplied by generated native startup.
pub use mod_points::__private::SlotListener;
