use crate::game::native_entity_refs::tests::{fixture_root, run_fresh_cases, with_client_game};
use mod_api::{
    ClientHost, ComponentTypesEvent, Hooks, ModEntry, Mods, Registrar, RegistrationContext, Text,
    TooltipAppendContext, TooltipAppender, TooltipPlacement,
};
use std::sync::Arc;
use sunrise_client::{items::ItemStack, text::Text as EngineText};
use sunrise_core::nbt::Tag;

struct Line(&'static str);
impl TooltipAppender for Line {
    fn append(
        &self,
        _: &dyn ClientHost,
        _: &mod_api::ItemStack,
        _: &TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    ) {
        lines.push((Text::literal(self.0), u32::MAX));
    }
}

struct First;
#[mod_api::__subscriptions]
impl Hooks for First {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        for (component, placement, line) in [
            (
                "minecraft:repair_cost",
                TooltipPlacement::Before("test:anchor".into()),
                "before mod anchor",
            ),
            (
                "minecraft:max_stack_size",
                TooltipPlacement::After("minecraft:lore".into()),
                "after lore constraint",
            ),
            (
                "minecraft:note_block_sound",
                TooltipPlacement::AfterAll,
                "missing component must not appear",
            ),
        ] {
            registrar.register_tooltip_appender(component, placement, Arc::new(Line(line)));
        }
    }
}

struct Second(bool);
#[mod_api::__subscriptions]
impl Hooks for Second {
    fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
        if self.0 {
            event.register("test:anchor".into());
        }
    }

    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        registrar.register_tooltip_appender(
            "test:anchor",
            TooltipPlacement::Before("minecraft:lore".into()),
            Arc::new(Line("mod anchor")),
        );
        registrar.register_tooltip_appender(
            "minecraft:max_damage",
            TooltipPlacement::Before("minecraft:damage".into()),
            Arc::new(Line("before damage")),
        );
        registrar.register_tooltip_appender(
            "minecraft:map_post_processing",
            TooltipPlacement::AfterAll,
            Arc::new(Line("transient component")),
        );
    }
}

#[test]
#[ignore = "daily"]
fn appended_lines_go_where_their_placement_puts_them() {
    const CASE: &str = "SUNRISE_TOOLTIP_APPENDER_CASE";
    if run_fresh_cases(
        "game::stations::tooltip_appender_tests::appended_lines_go_where_their_placement_puts_them",
        CASE,
        &["undeclared", "admitted"],
    ) {
        return;
    }
    let admitted = std::env::var(CASE).unwrap() == "admitted";
    let mods: Mods = vec![
        ModEntry::new("first", Box::new(First)),
        ModEntry::new("second", Box::new(Second(admitted))),
    ]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let result = crate::game::mods::register_components(&fixture_root());
        if !admitted {
            assert_eq!(
                result.unwrap_err().to_string(),
                "mod second: tooltip appender uses unregistered data component test:anchor"
            );
            return;
        }
        result.unwrap();
        assert!(sunrise_protocol::item::custom::id("test:anchor").is_some());
        with_client_game(|game| {
            let mut stack = ItemStack::new("minecraft:iron_sword", 1);
            let max = game.items().get(&stack.item).unwrap().max_damage;
            stack.damage = 1;
            let data = stack.data_mut();
            data.repair_cost = 1;
            data.set_other("minecraft:max_stack_size", Some(Tag::Int(1)));
            data.set_other(
                "minecraft:max_damage",
                Some(Tag::Int(i32::try_from(max).unwrap())),
            );
            data.set_other("test:anchor", Some(Tag::Int(1)));
            data.set_other(
                "minecraft:lore",
                Some(Tag::List(vec![Tag::compound([(
                    "text",
                    Tag::String("lore".into()),
                )])])),
            );
            // A transient component never written to the stack's saved patch.
            data.map_post_processing = Some(1);
            let durability = EngineText::translate(
                "item.durability",
                &[
                    EngineText::literal((max - 1).to_string()),
                    EngineText::literal(max.to_string()),
                ],
            )
            .plain();
            // Damage waits for its predecessor. The earlier ready After(lore) row wins
            // that tie; AfterAll still waits for damage and precedes the advanced ID.
            let expected = [
                "before mod anchor",
                "mod anchor",
                "lore",
                "after lore constraint",
                "before damage",
                durability.as_str(),
                "transient component",
                "minecraft:iron_sword",
            ];
            let witness = |stack: &ItemStack| {
                game.tooltip_lines(stack, false, false, true)
                    .into_iter()
                    .map(|(text, _)| text.plain())
                    .filter(|line| {
                        expected.contains(&line.as_str())
                            || line == "missing component must not appear"
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(witness(&stack), expected);
            stack.data_mut().set_other(
                "minecraft:tooltip_display",
                Some(Tag::compound([(
                    "hidden_components",
                    Tag::List(vec![Tag::String("test:anchor".into())]),
                )])),
            );
            assert_eq!(
                witness(&stack),
                [
                    "before mod anchor",
                    "lore",
                    "after lore constraint",
                    "before damage",
                    durability.as_str(),
                    "transient component",
                    "minecraft:iron_sword",
                ]
            );
            assert!(mod_api::__private::set_mod_enabled("second", false));
            assert_eq!(
                witness(&stack),
                [
                    "before mod anchor",
                    "lore",
                    "after lore constraint",
                    durability.as_str(),
                    "minecraft:iron_sword",
                ]
            );
            let old_anchor = mod_api::__private::tooltip_appenders()
                .iter()
                .find(|part| part.component() == "test:anchor")
                .unwrap();
            mod_platform::testing::with_mods(mods.clone(), || {
                // A new installation's enabled owner cannot resurrect an old callback.
                let slot = mod_api::__private::mod_index("second").unwrap();
                assert!(mod_api::__private::mod_index_on(slot));
                assert!(!old_anchor.enabled());
            });
        });
    });
}
