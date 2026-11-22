//! The `render_tooltip_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `RenderTooltipEvent.Pre`: a tooltip whose components have been prepared and are
    /// about to be measured, positioned and drawn. `stack` is the item the tooltip
    /// describes, or `None` for a non-item tooltip. `components` are the read-only
    /// prepared components (`PreparedTooltipComponent`, in display order, built-in and
    /// custom alike) as `render_tooltip_gather_components` and component preparation
    /// left them; measure them with a font borrowed via `draw.with_tooltip_font(font, ...)`
    /// or inspect them with `as_text`, `as_bundle`, and `downcast_ref`. `place` holds the
    /// anchor point in GUI pixels before `positioner` adds its offset and keeps the box on
    /// screen; cursor anchors truncate to whole GUI pixels, and a widget positioner may
    /// ignore the anchor. `screen` is `[screen_width, screen_height]` in GUI pixels.
    /// `positioner` is the active `ClientTooltipPositioner` that places the
    /// tooltip box once measured. `font` is the font identifier used for measurement and
    /// drawing (starting at `minecraft:default` or the item font queried again after Gather); change it to
    /// measure and draw with a replacement font without rewrapping already-split text
    /// lines. A prepared component's `width` and `height` return `None` on failure,
    /// distinct from a valid size of zero. A failed component measurement or draw
    /// aborts the whole tooltip. `draw` is the tooltip's GUI drawing context; paint
    /// from Pre stays visible on cancellation or failure. Return `Flow::Cancel` and the
    /// tooltip box and components are not drawn and `render_tooltip_texture` is not asked.
    /// Asked each frame a tooltip shows, only while a mod listens. Client-side.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        version = 3,
        contract = "cancel",
        out = "cancel",
        neoforge = "RenderTooltipEvent.Pre",
        cancellable = true
    )]
    fn render_tooltip_pre(
        stack: Option<&ItemStack>,
        components: &[mod_api::PreparedTooltipComponent],
        place: &mut TooltipPlace,
        screen: [f32; 2],
        positioner: &dyn mod_api::ClientTooltipPositioner,
        font: &mut Identifier,
        draw: &mut dyn DrawContext,
        cancel: &mut bool,
    );
}
