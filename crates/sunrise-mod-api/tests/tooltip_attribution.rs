#![cfg(feature = "testing")]
use mod_api::prelude::ItemsHost;
use mod_api::*;
use std::sync::Arc;

#[test]
fn tooltip_appender_restores_owner_even_on_panic() {
    struct Check(bool);
    impl TooltipAppender for Check {
        fn append(
            &self,
            _: &dyn ClientHost,
            _: &ItemStack,
            _: &TooltipAppendContext,
            lines: &mut Vec<(Text, u32)>,
        ) {
            assert_eq!(mod_api::content::crash::current_native_mod(), Some("inner"));
            lines.push((Text::literal("inner line"), u32::MAX));
            assert!(!self.0, "test unwind");
        }
    }
    struct Inner;
    #[mod_api::__subscriptions]
    impl Hooks for Inner {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            for (component, panic) in [("test:normal", false), ("test:panic", true)] {
                registrar.register_tooltip_appender(
                    component,
                    TooltipPlacement::AfterAll,
                    Arc::new(Check(panic)),
                );
            }
        }
    }
    let mods: Mods = vec![ModEntry::new("inner", Box::new(Inner))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let context = TooltipAppendContext {
            player: 0,
            player_entity_id: 0,
            tick_rate: 20.0,
            advanced: false,
            creative: false,
            nested: false,
        };
        let mut host = mod_api::testing::FakeHost::new();
        let id = Identifier::parse("minecraft:stick").unwrap();
        host.registries.resolve_items([id.clone()]).unwrap();
        let stack = ItemStack::of(host.item_key(&id).unwrap());
        let outer = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id: "outer",
            hook: "outer",
            part: None,
        });
        let appenders = mod_api::__private::tooltip_appenders();
        let [normal, failing] = appenders else {
            panic!("expected two appenders");
        };
        assert!(failing.enabled());
        assert!(failing.append(&host, &stack, &context).is_empty());
        assert!(!failing.enabled());
        assert_eq!(mod_api::content::crash::current_native_mod(), Some("outer"));
        assert_eq!(
            normal.append(&host, &stack, &context),
            [(Text::literal("inner line"), u32::MAX)]
        );
        assert_eq!(mod_api::content::crash::current_native_mod(), Some("outer"));
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            [mod_api::__private::HookPanicReport {
                mod_id: "inner",
                hook: "tooltip_appender",
            }]
        );
        drop(outer);
        assert_eq!(mod_api::content::crash::current_native_mod(), None);
    });
}
