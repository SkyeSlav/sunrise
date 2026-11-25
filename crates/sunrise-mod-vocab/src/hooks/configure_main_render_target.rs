//! Client main render target startup configuration (NeoForge's `ConfigureMainRenderTargetEvent`).

pub trait Hooks {
    /// Configure the main world render target once at client startup, before the renderer
    /// allocates its depth buffer (NeoForge's `ConfigureMainRenderTargetEvent`).
    ///
    /// Call `event.enable_stencil()` to request an 8-bit stencil attachment on the main world
    /// depth buffer (`Depth32FloatStencil8` when the GPU adapter supports
    /// `DEPTH32FLOAT_STENCIL8`, preserving 32-bit float reversed-Z depth precision, or
    /// `Depth24PlusStencil8` otherwise). When no mod enables stencil, the renderer keeps
    /// `Depth32Float` unchanged with zero overhead.
    ///
    /// Query whether stencil is active at draw time with
    /// [`ClientHost::main_target_stencil_enabled`], and reserve cooperative stencil bits with
    /// [`ClientHost::reserve_stencil_bit`] / [`ClientHost::release_stencil_bit`].
    #[notes = r#"## Main render target configuration & stencil

`Hooks::configure_main_render_target` is the client startup registration-phase counterpart of
NeoForge's `ConfigureMainRenderTargetEvent`. It fires once at client startup before the renderer
creates the main world depth buffer.

```rust,ignore
fn configure_main_render_target(
    &self,
    _context: &RegistrationContext,
    event: &mut ConfigureMainRenderTargetEvent,
) {
    event.enable_stencil();
}
```

When no mod calls `event.enable_stencil()`, the main world depth buffer remains `Depth32Float`
unchanged. When any mod enables stencil, the renderer prefers `Depth32FloatStencil8` when the
adapter supports `DEPTH32FLOAT_STENCIL8` (preserving 32-bit float reversed-Z depth precision)
and falls back to `Depth24PlusStencil8` otherwise (24-bit depth precision). The stencil buffer is
cleared to `0` at the start of each frame and survives window resizes and SDR/HDR transitions.

At draw time, check `host.main_target_stencil_enabled()` (or `context.main_target_stencil_enabled()`
in `client_setup`) and reserve cooperative stencil bits (`0..8`) with `host.reserve_stencil_bit()`
/ `host.release_stencil_bit(bit)` (NeoForge's `StencilManager`). Bits reserved by a mod are
automatically released if that mod is disabled or unloaded. Attach a [`StencilTest`] to a
[`RenderPipeline`] via `.stencil(...)` or to a [`PipelineModifier`] via `.stencil(...)`;
submitting a stencil-enabled pipeline to a target without a stencil attachment returns
[`PipelineDrawError::NoStencilAttachment`]."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        context = "registration",
        neoforge = "ConfigureMainRenderTargetEvent"
    )]
    fn configure_main_render_target(enable_stencil: &mut bool);
}
