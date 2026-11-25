use super::*;

impl Game<'_> {
    /// Collect the mods' custom feature renderers once at client startup, before loading
    /// resources (NeoForge 26.3's `RegisterFeatureRenderersEvent`). Duplicate registrations for
    /// the same `FeatureRendererType` identifier fail startup naming both mods; a panicking
    /// listener leaves earlier registrations unchanged.
    pub(in crate::game) fn native_register_feature_renderers(
        context: &mod_api::RegistrationContext,
        renderers: &mut mod_api::FeatureRendererRegistry,
    ) {
        Self::fire_register_feature_renderers(context, |listen| {
            renderers.register_listener(listen)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Game;
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{
        ClientHost, FeatureRegistrationError, FeatureRenderer, FeatureRendererRegistry,
        FeatureRendererType, Hooks, ModEntry, RegisterFeatureRenderersEvent, RegistrationContext,
        RenderPhaseKey, WorldDrawContext, WorldRenderView,
    };

    struct NodeA;
    struct NodeB;

    const TYPE_A: FeatureRendererType<NodeA> = FeatureRendererType::new("test:feature_a");
    const TYPE_B: FeatureRendererType<NodeB> = FeatureRendererType::new("test:feature_b");

    struct NoopA;
    impl FeatureRenderer<NodeA> for NoopA {
        fn render(
            &self,
            _: &dyn ClientHost,
            _: &WorldRenderView<'_>,
            _: RenderPhaseKey,
            _: &[NodeA],
            _: &mut dyn WorldDrawContext,
        ) {
        }
    }

    struct NoopB;
    impl FeatureRenderer<NodeB> for NoopB {
        fn render(
            &self,
            _: &dyn ClientHost,
            _: &WorldRenderView<'_>,
            _: RenderPhaseKey,
            _: &[NodeB],
            _: &mut dyn WorldDrawContext,
        ) {
        }
    }

    struct GoodMod;
    #[mod_api::__subscriptions]
    impl Hooks for GoodMod {
        fn register_feature_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterFeatureRenderersEvent<'_>,
        ) {
            event.renderers.register(&TYPE_A, NoopA);
        }
    }

    struct PanickingMod;
    #[mod_api::__subscriptions]
    impl Hooks for PanickingMod {
        fn register_feature_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterFeatureRenderersEvent<'_>,
        ) {
            event.renderers.register(&TYPE_B, NoopB);
            panic!("feature renderer registrar fixture failed");
        }
    }

    struct DuplicateMod;
    #[mod_api::__subscriptions]
    impl Hooks for DuplicateMod {
        fn register_feature_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterFeatureRenderersEvent<'_>,
        ) {
            event.renderers.register(&TYPE_A, NoopA);
        }
    }

    #[test]
    fn panicking_feature_renderer_registration_rolls_back_its_batch() {
        let mut registry = FeatureRendererRegistry::default();
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("good", Box::new(GoodMod)),
                ModEntry::new("panicker", Box::new(PanickingMod)),
            ]
            .into(),
            || {
                Game::native_register_feature_renderers(&RegistrationContext::new(), &mut registry);
            },
        );
        let table = registry.into_table().unwrap();
        assert!(table.get("test:feature_a").is_some());
        assert!(
            table.get("test:feature_b").is_none(),
            "panicking listener's registration must be rolled back"
        );
    }

    #[test]
    fn duplicate_feature_renderer_registration_names_both_mods() {
        let mut registry = FeatureRendererRegistry::default();
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("alpha", Box::new(GoodMod)),
                ModEntry::new("beta", Box::new(DuplicateMod)),
            ]
            .into(),
            || {
                Game::native_register_feature_renderers(&RegistrationContext::new(), &mut registry);
            },
        );
        let errors = registry.into_table().unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            FeatureRegistrationError::Duplicate {
                id: "test:feature_a",
                first_mod: "alpha".into(),
                second_mod: "beta".into(),
            }
        );
    }
}
