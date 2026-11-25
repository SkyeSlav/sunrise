mod api {
    #[derive(Debug, PartialEq, Eq)]
    pub struct EnvironmentEffectRendererRegistration(pub &'static str);
}

#[test]
fn register_custom_environment_effect_renderers_attributes_and_discards_failed_listener_tail() {
    struct Listener(bool);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register_custom_environment_effect_renderers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterCustomEnvironmentEffectRenderersEvent<'_>,
        ) {
            event
                .renderers
                .push(mod_api::mod_api::EnvironmentEffectRendererRegistration(
                    event.mod_id,
                ));
            if self.0 {
                panic!("the fixture's environment effect renderer listener unwinds");
            }
        }
    }

    install(
        vec![
            ModEntry::new("first", Box::new(Listener(false))),
            ModEntry::new("failed", Box::new(Listener(true))),
            ModEntry::new("second", Box::new(Listener(false))),
        ]
        .into(),
    );
    let mut renderers = Vec::new();
    dispatch_register_custom_environment_effect_renderers(
        &RegistrationContext("bootstrap"),
        &mut renderers,
    );
    assert_eq!(
        renderers,
        [
            mod_api::mod_api::EnvironmentEffectRendererRegistration("first"),
            mod_api::mod_api::EnvironmentEffectRendererRegistration("second"),
        ]
    );
}
