//! `GuiDraw::Item` for the hotbar and a mod's `gui_paint`: one path, `hud_layout::paint_items`,
//! with the local player's hotbar slots drawn as `extractSlot` draws them (`item`, then
//! `itemDecorations`). A mod's buffer is checked first and refused whole with the op it could
//! not draw.
use super::Game;
use mod_api::GuiPaintError;
use sunrise_client::hud::HudBatch;
use sunrise_client::hud_layout::{self, GuiDraw, GuiItems, Undrawable};
use sunrise_client::items::ItemStack;
use sunrise_items::equipment::OFFHAND;

/// Whose hotbar the slots come from: the local player, a live remote player, or a detached camera player.
#[derive(Clone, Copy)]
pub(crate) enum HotbarSource {
    Own,
    Mob(usize),
    DetachedCamera,
}

/// A hotbar's slots (0-8) and off hand (40), drawn for `GuiDraw::Item`.
pub(crate) struct HotbarItems<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) source: HotbarSource,
}
impl HotbarItems<'_, '_> {
    fn stack(&self, slot: u32) -> Option<&ItemStack> {
        match (self.source, slot) {
            (HotbarSource::Own, 40) => self.game.offhand.as_ref(),
            (HotbarSource::Own, 0..=8) => self.game.inventory.get(slot as usize)?.as_ref(),
            (HotbarSource::Mob(i), 40) => self.game.mobs[i].extra.slot_stack(OFFHAND),
            (HotbarSource::Mob(i), 0) => self.game.mobs[i].extra.main_hand_stack(),
            (HotbarSource::DetachedCamera, 40) => self
                .game
                .detached_camera_hud()
                .and_then(|hud| hud.offhand.as_ref()),
            (HotbarSource::DetachedCamera, 0) => self
                .game
                .detached_camera_hud()
                .and_then(|hud| hud.main_hand.as_ref()),
            _ => None,
        }
    }
}
impl GuiItems for HotbarItems<'_, '_> {
    fn item(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32, _seed: i32) {
        if let Some(stack) = self.stack(slot) {
            self.game.draw_item_model(batch, stack, x as f32, y as f32);
        }
    }
    fn decorations(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32) {
        if let Some(stack) = self.stack(slot) {
            self.game
                .draw_item_decorations(batch, stack, x as f32, y as f32, None);
        }
    }
}

/// The API's name for the op `paint_checked` refused at `index`.
pub(crate) fn paint_error((index, why): (usize, Undrawable)) -> GuiPaintError {
    let index = u32::try_from(index).unwrap_or(u32::MAX);
    match why {
        Undrawable::ItemSlot(slot) => GuiPaintError::ItemSlot { index, slot },
        Undrawable::ItemNotFinite => GuiPaintError::ItemNotFinite { index },
        Undrawable::BlitTexture(texture) => GuiPaintError::BlitTexture { index, texture },
        Undrawable::BlitSize => GuiPaintError::BlitSize { index },
        Undrawable::Sprite(sprite) => GuiPaintError::Sprite { index, sprite },
        Undrawable::SpriteRegion => GuiPaintError::SpriteRegion { index },
        Undrawable::SpriteBlend(blend) => GuiPaintError::SpriteBlend { index, blend },
    }
}

impl Game<'_> {
    /// A mod's `gui_paint` buffer: every op drawn under its pose, items from the local player's
    /// hotbar, or nothing and the op it could not draw.
    pub(crate) fn native_gui_paint(
        &self,
        batch: &mut HudBatch,
        draws: &[GuiDraw],
    ) -> Result<(), GuiPaintError> {
        let mut items = HotbarItems {
            game: self,
            source: HotbarSource::Own,
        };
        hud_layout::paint_checked(batch, &self.gui.atlas, draws, &mut items).map_err(paint_error)
    }
}
