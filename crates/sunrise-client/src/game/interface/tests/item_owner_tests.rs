use super::{Dim, DisplayContext, ItemStack, OwnerState, living_owner_at, owner_at};
use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use serde_json::{Value, json};
use sunrise_assets::item_model::{ClientItem, Context, ItemModels, LayerContent, NoEnv, StackView};

fn model_fixture(model: Value) -> (ItemModels, StackView) {
    let mut models = ItemModels::default();
    let item = ClientItem::from_json(&json!({"model": model}).to_string()).unwrap();
    models.definitions.insert("test:probe".into(), item);
    let mut stack = StackView::new("test:probe", 1);
    stack
        .patch
        .insert("minecraft:item_model".into(), Some(json!("test:probe")));
    (models, stack)
}

#[test]
fn living_only_item_selects_fall_back_for_dropped_and_framed_items() {
    let leaf = |id| json!({"type": "minecraft:model", "model": id});
    let (models, stack) = model_fixture(json!({
        "type": "minecraft:composite",
        "models": [
            {
                "type": "minecraft:select",
                "property": "minecraft:main_hand",
                "cases": [
                    {"when": "left", "model": leaf("test:left")},
                    {"when": "right", "model": leaf("test:right")}
                ],
                "fallback": leaf("test:no_hand")
            },
            {
                "type": "minecraft:select",
                "property": "minecraft:context_entity_type",
                "cases": [
                    {"when": "minecraft:item", "model": leaf("test:item_entity")},
                    {"when": "minecraft:item_frame", "model": leaf("test:item_frame")},
                    {"when": "minecraft:player", "model": leaf("test:player")},
                    {"when": "minecraft:zombie", "model": leaf("test:zombie")}
                ],
                "fallback": leaf("test:no_entity")
            }
        ]
    }));
    for (owner, display, hand, entity) in [
        (
            owner_at([0.0; 3], 0.0, "minecraft:item"),
            DisplayContext::Ground,
            "test:no_hand",
            "test:no_entity",
        ),
        (
            owner_at([0.0; 3], 0.0, "minecraft:item_frame"),
            DisplayContext::Fixed,
            "test:no_hand",
            "test:no_entity",
        ),
        (
            living_owner_at([0.0; 3], 0.0, "minecraft:player", true, None),
            DisplayContext::Gui,
            "test:left",
            "test:player",
        ),
        (
            living_owner_at([0.0; 3], 0.0, "minecraft:zombie", false, None),
            DisplayContext::ThirdPersonRightHand,
            "test:right",
            "test:zombie",
        ),
    ] {
        let ctx = Context {
            display,
            owner: Some(&owner),
            level: None,
            seed: 0,
        };
        let resolved = models.resolve(&stack, &ctx, &NoEnv);
        assert_eq!(
            resolved
                .layers
                .into_iter()
                .map(|layer| layer.content)
                .collect::<Vec<_>>(),
            [hand, entity].map(|model| LayerContent::Model {
                model: model.to_owned()
            }),
            "{}",
            owner.entity_type
        );
    }
}

#[test]
#[ignore = "daily"]
fn item_team_tint_uses_the_living_owners_scoreboard_color() {
    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_team_tint_uses_the_living_owners_scoreboard_color",
        || {
            with_game(|game| {
                let (models, stack) = model_fixture(json!({
                    "type": "minecraft:model",
                    "model": "test:tinted",
                    "tints": [{"type": "minecraft:team", "default": 0x12_3456}]
                }));
                let tint = |owner: &OwnerState| {
                    let ctx = Context {
                        display: DisplayContext::Gui,
                        owner: Some(owner),
                        level: None,
                        seed: 0,
                    };
                    models.resolve(&stack, &ctx, &NoEnv).layers[0].tints[0] as u32
                };
                let player_name = game.acting_player_name().to_owned();
                assert_eq!(tint(&game.player_owner(None)), 0xff12_3456);
                game.score.board.add_team("test");
                assert!(game.score.board.add_to_team(&player_name, "test"));
                assert_eq!(tint(&game.player_owner(None)), 0xff12_3456);
                game.score.board.team_mut("test").unwrap().color = Some("red".into());
                assert_eq!(tint(&game.player_owner(None)), 0xffff_5555);

                let id = game
                    .spawn_mob_raw("minecraft:zombie", [0.5, 64.0, 0.5], true)
                    .unwrap();
                let holder = sunrise_core::uuid::uuid_text(game.persist.uuid_of(id).unwrap());
                assert!(game.score.board.add_to_team(&holder, "test"));
                let mob = game.mobs.iter().find(|mob| mob.id == id).unwrap();
                assert_eq!(tint(&game.mob_item_owner(mob, false)), 0xffff_5555);

                // A non-living OwnerState must not supply a team to the tint source.
                let mut frame = owner_at([0.0; 3], 0.0, "minecraft:item_frame");
                frame.team_color = Some(0xffff_5555_u32 as i32);
                assert_eq!(tint(&frame), 0xff12_3456);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_draw_uses_the_stack_duration_even_when_its_model_is_replaced() {
    use sunrise_core::nbt::Tag;

    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_draw_uses_the_stack_duration_even_when_its_model_is_replaced",
        || {
            with_client_game(|game| {
                let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                horn.data_mut().set_other(
                    "minecraft:instrument",
                    Some(Tag::compound([
                        (
                            "sound_event",
                            Tag::String("minecraft:item.goat_horn.sound.0".into()),
                        ),
                        ("use_duration", Tag::Float(2.37_f32.to_bits())),
                        ("range", Tag::Float(16.0_f32.to_bits())),
                        ("description", Tag::String("Test horn".into())),
                    ])),
                );
                let mut bow = ItemStack::new("minecraft:bow", 1);
                bow.data_mut().set_other(
                    "minecraft:item_model",
                    Some(Tag::String("test:bow_model".into())),
                );
                for (stack, duration) in [
                    (ItemStack::new("minecraft:spyglass", 1), 1200),
                    (ItemStack::new("minecraft:bundle", 1), 200),
                    (horn, 47),
                    (bow, 72_000),
                    (ItemStack::new("minecraft:shield", 1), 72_000),
                    (ItemStack::new("minecraft:bread", 1), 32),
                    (ItemStack::new("minecraft:stick", 1), 0),
                ] {
                    game.inventory[game.selected] = Some(stack.clone());
                    game.using = Some((game.selected, 0));
                    let owner = game.player_owner(Some(&stack));
                    assert!(owner.using_this_item);
                    assert_eq!(owner.use_duration, duration, "{}", stack.item);
                }
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn item_moon_phase_follows_the_dimension_timeline() {
    use std::sync::Arc;
    use sunrise_client::environment::Environment;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C},
    };

    crate::game::in_own_process(
        "game::item_draw::owner_tests::item_moon_phase_follows_the_dimension_timeline",
        || {
            with_client_game(|game| {
                game.world.day_time = 24_000 * 3;
                for (dim, phase) in [(Dim::Nether, 0), (Dim::End, 0), (Dim::Overworld, 3)] {
                    game.dim = dim;
                    assert_eq!(game.item_level().moon_phase, phase, "{}", dim.id());
                }

                // A server can give another level the moon timeline. Its clock is
                // the Overworld's, even when the level's day time follows a different one.
                let environment = Environment::with_type(
                    game.res.environment_of(Dim::End.id()),
                    &json!({"timelines": ["minecraft:moon"]}),
                )
                .unwrap();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "MoonTimeline").unwrap();
                let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
                registries.registries.insert(
                    "minecraft:world_clock".into(),
                    vec![("minecraft:overworld".into(), None)],
                );
                game.set_registries(Arc::new(registries));
                game.handle_packet(C::SetTime(play::SetTime {
                    game_time: 100,
                    clocks: vec![play::ClockState {
                        clock: 0,
                        total_ticks: 24_000 * 3,
                        partial_tick: 0.0,
                        rate: 1.0,
                    }],
                }))
                .unwrap();
                game.dim = Dim::End;
                game.world.day_time = 24_000 * 6;
                game.remote.as_mut().unwrap().level_environment = Some(Arc::new(environment));
                assert_eq!(game.item_level().moon_phase, 3);
            })
        },
    );
}
