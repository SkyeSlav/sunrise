use super::*;
use sunrise_items::creative_catalog::Catalog;

/// The local player's `TickThrottler(20, 1280)`.
#[derive(Clone, Copy, Default)]
pub struct DropSpamThrottler(i32);

impl DropSpamThrottler {
    pub fn tick(&mut self) {
        if self.0 > 0 {
            self.0 -= 1;
        }
    }

    pub fn is_under_threshold(self) -> bool {
        self.0 < 1280
    }

    pub fn increment(&mut self) {
        self.0 = self.0.wrapping_add(20);
    }
}

/// `doClick` must see which takes would send a creative drop before trying the
/// next take. Keep this state in the scratch menu so a discarded prediction
/// sends nothing and never advances the live player's throttler.
#[derive(Clone)]
pub(super) struct ClientDropPrediction {
    throttle: DropSpamThrottler,
    creative: Option<(Arc<Catalog>, Arc<[String]>)>,
    throws: Vec<ItemStack>,
}

impl Menu {
    /// Predict against the local player's counter. The creative catalog and
    /// flags are passed only when `handleCreativeModeItemDrop` may send.
    pub fn predict_client_drops(
        &mut self,
        throttle: DropSpamThrottler,
        creative: Option<(Arc<Catalog>, Arc<[String]>)>,
    ) {
        self.touch();
        self.state.client_drops = Some(ClientDropPrediction {
            throttle,
            creative,
            throws: Vec::new(),
        });
    }

    /// Carry the predicted counter across successive scratch menus in one batch.
    pub fn predicted_drop_spam(&self) -> Option<DropSpamThrottler> {
        self.state.client_drops.as_ref().map(|drops| drops.throttle)
    }

    /// Creative drops for this click, to send before that click's own packet.
    pub fn take_predicted_creative_throws(&mut self) -> Vec<ItemStack> {
        self.touch();
        self.state
            .client_drops
            .as_mut()
            .map_or_else(Vec::new, |drops| std::mem::take(&mut drops.throws))
    }

    pub(super) fn can_throw(&self) -> bool {
        self.predicted_drop_spam()
            .is_none_or(DropSpamThrottler::is_under_threshold)
    }

    pub(super) fn throw_stack(&mut self, taken: &ItemStack, items: &Items) {
        if taken.is_empty() {
            return;
        }
        self.drop_stack(taken, items);
        if let Some(drops) = &mut self.state.client_drops
            && let Some((catalog, flags)) = &drops.creative
            && catalog.is_item_enabled(&taken.item, flags)
        {
            drops.throws.push(taken.clone());
            drops.throttle.increment();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crafting::Ingredient, stations::Stonecutting};

    #[test]
    fn drop_spam_burst_and_tick_recovery() {
        let mut throttle = DropSpamThrottler::default();
        throttle.tick(); // Idle ticks do not accumulate credit.
        for _ in 0..64 {
            assert!(throttle.is_under_threshold());
            throttle.increment();
        }
        assert!(!throttle.is_under_threshold());
        assert_eq!(throttle.0, 1280);
        throttle.tick();
        assert!(throttle.is_under_threshold());
        throttle.increment();
        assert_eq!(throttle.0, 1299);
        for _ in 0..19 {
            assert!(!throttle.is_under_threshold());
            throttle.tick();
        }
        assert_eq!(throttle.0, 1280);
        assert!(!throttle.is_under_threshold());
        throttle.tick();
        assert_eq!(throttle.0, 1279);
        assert!(throttle.is_under_threshold());
    }

    #[test]
    fn ctrl_q_rechecks_drop_spam_before_each_replenished_take() {
        super::super::tests::bare_env(|env| {
            // Each take refills the result with the same item, like the repeat
            // slot in SyntheticContainerDrop. Five inputs allow five takes.
            let input = ItemStack::new("minecraft:stone", 5);
            let result = ItemStack::new("minecraft:stone_slab", 1);
            let recipes = env.station_recipes.with_stonecutting(vec![Stonecutting {
                id: "test:cut_stone".into(),
                input: Ingredient::of(vec![input.item.to_string()]),
                result: result.clone(),
            }]);
            let env = Env {
                station_recipes: &recipes,
                ..*env
            };
            let mut menu = Menu::new(Kind::Stonecutter);
            menu.set_slot(&env, 0, Some(input));
            assert!(menu.button(&env, 0));
            assert_eq!(menu.slot(1), Some(&result));

            let mut throttle = DropSpamThrottler::default();
            for _ in 0..62 {
                throttle.increment();
            }
            let catalog = Catalog::parse(&serde_json::json!({
                "schema": 4, "tabs": [], "potion_features": {},
                "item_features": {"minecraft:stone_slab": ["minecraft:vanilla"]}
            }))
            .unwrap();
            menu.predict_client_drops(
                throttle,
                Some((Arc::new(catalog), ["minecraft:vanilla".to_owned()].into())),
            );
            menu.click(&env, 1, 1, Input::Throw).unwrap();
            assert_eq!(
                menu.take_predicted_creative_throws(),
                [result.clone(), result.clone()]
            );
            assert_eq!(menu.slot(0).map(|s| s.count), Some(3));
            assert_eq!(menu.slot(1), Some(&result));

            menu.click(&env, 1, 0, Input::Throw).unwrap();
            assert_eq!(menu.slot(0).map(|s| s.count), Some(3));
            assert_eq!(menu.slot(1), Some(&result));
            assert!(menu.take_predicted_creative_throws().is_empty());
        });
    }
}
