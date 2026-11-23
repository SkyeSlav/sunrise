//! Dynamic textures, inline images, HUD overlay messages, living scenes and screen helpers.

pub mod dynamic_panel {
    // ANCHOR: dynamic_panel
    use mod_api::{ClientHost, DynamicTextureError, GuiRenderPostEvent, Hooks, Identifier};

    const PANEL: &str = "mymod:dynamic/panel";

    /// Bake the panel once: 64 by 32 pixels, row by row from the top left, four bytes (RGBA) each.
    pub fn bake_panel(host: &mut dyn ClientHost) -> Result<(), DynamicTextureError> {
        let (width, height) = (64, 32);
        let rgba = [24, 24, 48, 230].repeat(width * height);
        let id = Identifier::parse(PANEL).unwrap();
        host.create_dynamic_texture(&id, width as u32, height as u32, &rgba)
    }

    #[derive(Default)]
    struct PanelHud;

    #[mod_api::sunrise_mod]
    impl Hooks for PanelHud {
        fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
            // The whole image (`src` in its own pixels), stretched over 128 by 64 GUI pixels.
            event
                .batch
                .sprite(PANEL, [0.0, 0.0, 64.0, 32.0], [8.0, 8.0, 128.0, 64.0], None);
        }
    }
    // ANCHOR_END: dynamic_panel
}

pub mod inline_images {
    // ANCHOR: inline_images
    use mod_api::{
        ClientHost, Hooks, Identifier, InlineImageReplacement, SubstituteInlineImagesEvent,
    };

    #[derive(Default)]
    struct StarImages;

    #[mod_api::sunrise_mod]
    impl Hooks for StarImages {
        fn substitute_inline_images(
            &self,
            _: &dyn ClientHost,
            event: &mut SubstituteInlineImagesEvent<'_>,
        ) {
            let sprite = Identifier::parse("mymod:dynamic/star").unwrap();
            for (start, matched) in event.run.match_indices(":star:") {
                let (Ok(start), Ok(end)) =
                    (u32::try_from(start), u32::try_from(start + matched.len()))
                else {
                    continue;
                };
                event.images.push(InlineImageReplacement {
                    start,
                    end,
                    sprite: sprite.clone(),
                });
            }
        }
    }
    // ANCHOR_END: inline_images
}

pub mod hud_overlay {
    // ANCHOR: tool_mode_changed
    use mod_api::{ClientHost, Text};

    pub fn tool_mode_changed(host: &mut dyn ClientHost, mode: &str) {
        let line = Text::translate("message.example.tool_mode", vec![Text::literal(mode)]);
        if let Err(error) = host.set_overlay_message(line, false) {
            log::debug!("action bar refused: {error:?}");
        }
    }
    // ANCHOR_END: tool_mode_changed
}

pub mod living_scenes {
    use mod_api::{DrawContext, SceneBlock, SceneError};

    pub fn draw_example(
        draw: &mut dyn DrawContext,
        rect: [f32; 4],
        blocks: Vec<SceneBlock>,
        view: mod_api::SceneView,
        page_ticks: u64,
        partial: f32,
    ) -> Result<(), SceneError> {
        // ANCHOR: draw_living_scene
        use mod_api::{GuiEntity, Scene, SceneClock, SceneEntity, SceneView};

        // Blocks alone: no clock, no state.
        draw.draw_scene(rect, &Scene::new(blocks.clone()), view)?;

        // In the menu screen factory: build a living scene once and move it into the callback.
        let mut scene = Scene::new(blocks)
            .with_clock(SceneClock::new(1, 0, 0.0))
            .with_entities(vec![
                SceneEntity::new(GuiEntity::kind("minecraft:cow"), [1.5, 0.0, 0.5])
                    .turned(135.0, 150.0, 0.0),
            ]);

        // In the draw callback: update the clock from the page's own ticks (20 per second)
        // and partial tick.
        if let Some(clock) = &mut scene.clock {
            clock.tick = page_ticks;
            clock.partial = partial;
        }
        let view = SceneView::new([1.0, 0.5, 0.5], 225.0, 30.0, 18.0);
        if let Err(why) = draw.draw_scene([12.0, 20.0, 140.0, 110.0], &scene, view) {
            log::warn!("scene not drawn whole: {why:?}");
        }
        // ANCHOR_END: draw_living_scene
        Ok(())
    }
}

pub mod screens {
    use mod_api::client::hud_layout::{GuiDraw, PoseOp};
    use mod_api::{ClientHost, DrawContext, Text, TextAlign};

    pub fn screen_controls(host: &mut dyn ClientHost) {
        // ANCHOR: screen_controls
        // Escape: close whatever is open.
        if host.close_screen() {
            // a screen was open and is now closed
        }

        // The inventory key, key.inventory.
        if host.open_inventory() {
            // the inventory screen is up (or the ridden vehicle's screen was asked for)
        }

        // Which sign the editor is on, if any.
        if let Some(sign) = host.sign_editor() {
            let _ = (sign.pos, sign.front);
        }
        // ANCHOR_END: screen_controls
    }

    pub fn draw_scaled_label(
        draw: &mut dyn DrawContext,
        label: Text,
        x: f32,
        y: f32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // ANCHOR: scaled_text_lines
        // From a menu screen draw callback, gui_render_post or another drawing hook.
        let scale = 2.0;
        let lines = draw.text_lines(&label, 120.0, TextAlign::Centre, scale)?;
        let mut draws = Vec::with_capacity(lines.lines.len() + 2);
        draws.push(GuiDraw::Push(vec![PoseOp::Scale(scale, scale)]));
        for line in &lines.lines {
            draws.push(GuiDraw::Text {
                text: line.text.clone(),
                x: (x + line.at[0] / scale) as i32,
                y: (y + line.at[1] / scale) as i32,
                color: 0xFFFF_FFFF,
                shadow: false,
            });
        }
        draws.push(GuiDraw::Pop);
        draw.gui_paint(&draws)?;
        // lines.width and lines.height size the box you reserved for it.
        // ANCHOR_END: scaled_text_lines
        Ok(())
    }
}
