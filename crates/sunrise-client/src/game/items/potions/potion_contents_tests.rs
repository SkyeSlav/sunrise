//! `AbstractThrownPotion.onHit` reads the stack's whole `potion_contents`: the
//! potion's effects and the stack's `custom_effects`, the colour they mix to
//! (or `custom_color`) and its `potion_duration_scale`.
use super::*;
use crate::game::game_tests::tests::server_game;
use sunrise_client::motion::PlayerMotion;

/// A server's game with a player standing at the origin's block.
fn player_game(resources: &mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'_> {
    let mut game = server_game(resources);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    // A fixed name: no generated sunrise-player.txt in the working directory.
    game.options.player_name = Some("PotionContents".into());
    game.build_registries();
    game.set_allow_commands(true);
    game
}

/// A stack with `custom_effects` of `(effect, duration, amplifier)` on top of
/// whatever potion it carries, as `PotionContents.customEffects`.
fn with_custom_effects(mut stack: ItemStack, effects: &[(&str, i32, i32)]) -> ItemStack {
    let list = effects
        .iter()
        .map(|(id, duration, amp)| {
            Tag::compound([
                ("id", Tag::String(format!("minecraft:{id}").as_str().into())),
                ("duration", Tag::Int(*duration)),
                ("amplifier", Tag::Int(*amp)),
            ])
        })
        .collect();
    stack
        .data_mut()
        .potion_extra
        .push(("custom_effects".into(), Tag::List(list)));
    stack
}

/// `ThrownSplashPotion.onHitAsPotion` over `PotionContents.getAllEffects` and
/// `PotionContents.getColor`: a swiftness potion carrying a custom poison
/// effect lands both, each for `(int)(scale * d * durationScale + 0.5)` of its
/// ticks, and the splash's hundred spell particles are drawn in the contents'
/// `custom_color`.
#[test]
#[ignore = "daily"]
fn a_splash_potion_applies_its_custom_effects_and_duration_scale() {
    let mut resources = None;
    let mut game = player_game(&mut resources);
    let mut stack = with_custom_effects(
        ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:swiftness"),
        &[("poison", 900, 0)],
    );
    stack.data_mut().set_other(
        "minecraft:potion_duration_scale",
        Some(Tag::Float(0.125f32.to_bits())),
    );
    stack
        .data_mut()
        .potion_extra
        .push(("custom_color".into(), Tag::Int(0x0020_4080)));
    game.particles.requested = Some(Vec::new());
    let at = Vec3::new(0.5, 64.0, 0.5);
    game.splash(
        &stack,
        at,
        None,
        None,
        0,
        &PotionOwner::default(),
        None,
        true,
        Vec3::ZERO,
    );
    let effects = &game.potions.effects;
    // The potion's 3600 ticks and the custom effect's 900, both at 0.125.
    assert_eq!(
        effects.get("speed").map(|e| e.duration),
        Some(450),
        "the potion's own effect, scaled"
    );
    assert_eq!(
        effects.get("poison").map(|e| e.duration),
        Some(113),
        "the custom effect, scaled"
    );
    let spell: Vec<[f32; 4]> = game
        .particles
        .requested
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(o, ..)| o.ty == ParticleType::Effect)
        .map(|(o, ..)| o.color)
        .collect();
    assert_eq!(spell.len(), 100, "the splash's spell particles");
    for c in spell {
        // 0x204080, brightened by each particle's own 0.75 to 1.
        assert!(
            (c[0] * 2.0 - c[1]).abs() < 1.0e-4 && (c[1] * 2.0 - c[2]).abs() < 1.0e-4,
            "{c:?}"
        );
    }
}

/// `ThrownLingeringPotion.onHitAsPotion` with `AreaEffectCloud.applyComponentsFromItemStack`:
/// contents with only custom effects still make a cloud, in their colour, and
/// it sprays them at the stack's `potion_duration_scale` (1.0 here, over the
/// lingering potion's own 0.25).
#[test]
#[ignore = "daily"]
fn a_lingering_potion_of_custom_effects_makes_its_cloud() {
    let mut resources = None;
    let mut game = player_game(&mut resources);
    let mut stack = with_custom_effects(
        ItemStack::new("minecraft:lingering_potion", 1),
        &[("poison", 900, 0)],
    );
    stack
        .data_mut()
        .potion_extra
        .push(("custom_color".into(), Tag::Int(0x0012_3456)));
    stack.data_mut().set_other(
        "minecraft:potion_duration_scale",
        Some(Tag::Float(1.0f32.to_bits())),
    );
    game.splash(
        &stack,
        Vec3::new(0.5, 64.0, 0.5),
        None,
        None,
        0,
        &PotionOwner::default(),
        None,
        true,
        Vec3::ZERO,
    );
    assert_eq!(
        game.breath_clouds.len(),
        1,
        "a cloud for custom-only contents"
    );
    let cloud = game.breath_clouds[0].potion.as_ref().unwrap();
    assert_eq!(cloud.effects, vec![("poison", 900, 0)]);
    assert_eq!(cloud.settings.duration_scale, 1.0);
    let particle = cloud.particle();
    assert_eq!(particle.ty, ParticleType::EntityEffect);
    assert_eq!(
        particle.color,
        [
            0x12 as f32 / 255.0,
            0x34 as f32 / 255.0,
            0x56 as f32 / 255.0,
            1.0
        ],
        "the cloud's particles are drawn in the contents' colour"
    );
}

/// `AbstractThrownPotion.affectEntitiesAround` with `Axolotl.rehydrate`:
/// water refills an axolotl's air by 1800 once, not once per copy of the
/// `#rehydrates_axolotls` check.
#[test]
#[ignore = "daily"]
fn a_water_potion_refills_an_axolotl_once() {
    let mut resources = None;
    let mut game = player_game(&mut resources);
    assert!(game.run_command(
        "summon minecraft:axolotl 0.5 64 0.5 {NoAI:1b,PersistenceRequired:1b,Silent:1b}"
    ));
    game.mobs[0].air = 0;
    let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
    game.splash(
        &water,
        Vec3::new(0.5, 64.0, 0.5),
        None,
        None,
        0,
        &PotionOwner::default(),
        None,
        true,
        Vec3::ZERO,
    );
    assert_eq!(game.mobs[0].air, 1800, "one rehydrate, capped at the max");
}

/// `PotionContents.is(PotionTags.DOUSES_FIRE)` needs `isPotionWithoutCustomEffects`:
/// water carrying a custom effect leaves a lit campfire alone, while plain
/// water still puts it out.
#[test]
#[ignore = "daily"]
fn a_water_potion_with_custom_effects_does_not_douse_fire() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let pos = [2, 4, 3];
    let campfire = game
        .pack
        .assets
        .registry
        .default_state("minecraft:campfire")
        .unwrap();
    game.world.set(pos[0], pos[1], pos[2], campfire).unwrap();
    let lit = |game: &Game<'_>| {
        game.pack
            .assets
            .registry
            .state(game.world.block(pos[0], pos[1], pos[2]))
            .and_then(|s| s.properties.get("lit"))
            .is_some_and(|v| v == "true")
    };
    assert!(lit(&game), "the campfire starts lit");
    // The potion hits the top of the block under the campfire, so the cell
    // `AbstractThrownPotion.douseFire` looks at first is the campfire's.
    let hit = Some(([2, 3, 3], Direction::Up));
    let at = Vec3::new(2.5, 3.5, 3.5);
    let throw = |game: &mut Game<'_>, stack: &ItemStack| {
        game.splash(
            stack,
            at,
            None,
            hit,
            0,
            &PotionOwner::default(),
            None,
            true,
            Vec3::ZERO,
        )
    };
    let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
    throw(
        &mut game,
        &with_custom_effects(water.clone(), &[("slowness", 1800, 0)]),
    );
    assert!(
        lit(&game),
        "water with custom effects is not in #douses_fire"
    );
    throw(&mut game, &water);
    assert!(!lit(&game), "plain water puts the campfire out");
}

/// Vanilla 26.3 `AreaEffectCloud.serverTick`, `ThrownSplashPotion.onHitAsPotion`, and
/// `AbstractThrownPotion.affectEntitiesAround` pass the cloud or potion entity itself as the
/// direct entity of `DamageSources.indirectMagic(direct, owner)` before `Enchantment.damageContext`
/// evaluates worn `minecraft:damage_immunity` predicates on `direct_attacker`.
#[test]
#[ignore = "daily"]
fn live_cloud_and_splash_potion_carry_direct_entity_to_worn_immunity() {
    use crate::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood;

    crate::game::in_own_process(
        "game::potions::potion_contents_tests::live_cloud_and_splash_potion_carry_direct_entity_to_worn_immunity",
        || {
            let mut resources = None;
            let mut game = player_game(&mut resources);
            let root = tempfile::tempdir().unwrap();
            let ench_dir = root
                .path()
                .join("datapacks/direct_guard/data/test/enchantment");
            std::fs::create_dir_all(&ench_dir).unwrap();
            std::fs::write(
                root.path().join("datapacks/direct_guard/pack.mcmeta"),
                r#"{"pack":{"description":"Direct guard fixture"}}"#,
            )
            .unwrap();
            for (name, direct_type) in [
                ("cloud_guard", "minecraft:area_effect_cloud"),
                ("splash_guard", "minecraft:splash_potion"),
            ] {
                let def = serde_json::json!({
                    "description": name,
                    "supported_items": "minecraft:leather_boots",
                    "weight": 1,
                    "max_level": 1,
                    "anvil_cost": 1,
                    "slots": ["feet"],
                    "min_cost": { "base": 1, "per_level_above_first": 0 },
                    "max_cost": { "base": 1, "per_level_above_first": 0 },
                    "effects": {
                        "minecraft:damage_immunity": [{
                            "effect": {},
                            "requirements": {
                                "type": "minecraft:entity_properties",
                                "entity": "direct_attacker",
                                "predicate": { "minecraft:entity_type": direct_type }
                            }
                        }]
                    }
                });
                std::fs::write(
                    ench_dir.join(format!("{name}.json")),
                    serde_json::to_vec(&def).unwrap(),
                )
                .unwrap();
            }
            std::fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["vanilla","file/direct_guard"],"disabled":[]}"#,
            )
            .unwrap();
            game.world_root = root.path().to_path_buf();
            let packs = game.reload_datapacks(false, &[]).unwrap();
            let res_path = game.options.resources.clone();
            let staged = game.stage_world_tables(&packs, false, &res_path).unwrap();
            game.world_tables = staged.tables;
            game.health_resources = staged.health;
            prepare_entity_ticking_neighborhood(
                &mut game,
                sunrise_core::position::ChunkPos::new(0, 0),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 0..8 {
                for z in 0..8 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let boots = |enchant: &str| {
                let mut stack = ItemStack::new("minecraft:leather_boots", 1);
                stack.set_crafting_enchantments(vec![(enchant.into(), 1)]);
                stack
            };
            let summon_guarded = |game: &mut Game<'_>, mob: &str, x: f64, z: f64, enchant: &str| {
                let cmd = format!(
                    concat!(
                        "summon {mob} {x} 64 {z} {{NoAI:1b,NoGravity:1b,",
                        "equipment:{{feet:{{id:\"minecraft:leather_boots\",count:1,components:",
                        "{{\"minecraft:enchantments\":{{\"{enchant}\":1}}}}}}}}}}"
                    ),
                    mob = mob,
                    x = x,
                    z = z,
                    enchant = enchant
                );
                assert!(game.run_command(&cmd), "summon failed: {cmd}");
                game.mobs.last().unwrap().id
            };

            // Keep the player out of range during mob cases so clouds do not shrink on them.
            game.player.as_mut().unwrap().position = Vec3::new(20.5, 64.0, 20.5);

            // 1. Live lingering cloud (`tick_potion_clouds`): cloud_guard blocks, splash_guard takes damage.
            let cloud_immune =
                summon_guarded(&mut game, "minecraft:pig", 2.5, 2.5, "test:cloud_guard");
            let cloud_hit =
                summon_guarded(&mut game, "minecraft:pig", 2.5, 2.5, "test:splash_guard");
            let lingering =
                ItemStack::new("minecraft:lingering_potion", 1).with_potion("minecraft:harming");
            let mut linger_proj =
                thrown_projectile(lingering.clone(), Vec3::new(2.5, 64.1, 2.5), true);
            linger_proj.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(linger_proj);
            game.tick_projectiles().unwrap();
            assert_eq!(
                game.breath_clouds.len(),
                1,
                "lingering impact spawns a live cloud"
            );
            // Advance past the cloud's 10-tick wait time (`AreaEffectCloud.waitTime`).
            for _ in 0..11 {
                game.tick_potion_clouds();
            }
            let immune_pig = game.mobs.iter().find(|m| m.id == cloud_immune).unwrap();
            let hit_pig = game.mobs.iter().find(|m| m.id == cloud_hit).unwrap();
            assert_eq!(
                (immune_pig.health, immune_pig.hurt_time),
                (10.0, 0),
                "cloud_guard blocks live lingering cloud damage"
            );
            assert!(
                hit_pig.health < 10.0 && hit_pig.hurt_time > 0,
                "mismatched splash_guard still takes live lingering cloud damage"
            );
            game.breath_clouds.clear();
            game.mobs.clear();

            // 2. Live thrown splash potion via `tick_projectiles` (`ProjectileKind::Potion`).
            let splash_immune =
                summon_guarded(&mut game, "minecraft:pig", 4.5, 2.5, "test:splash_guard");
            let splash_hit =
                summon_guarded(&mut game, "minecraft:pig", 4.5, 2.5, "test:cloud_guard");
            let harming =
                ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:harming");
            let mut splash_proj =
                thrown_projectile(harming.clone(), Vec3::new(4.5, 64.1, 2.5), true);
            splash_proj.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(splash_proj);
            game.tick_projectiles().unwrap();
            let immune_pig = game.mobs.iter().find(|m| m.id == splash_immune).unwrap();
            let hit_pig = game.mobs.iter().find(|m| m.id == splash_hit).unwrap();
            assert_eq!(
                (immune_pig.health, immune_pig.hurt_time),
                (10.0, 0),
                "splash_guard blocks live ProjectileKind::Potion splash damage"
            );
            assert!(
                hit_pig.health < 10.0 && hit_pig.hurt_time > 0,
                "mismatched cloud_guard still takes live ProjectileKind::Potion splash damage"
            );
            game.mobs.clear();

            // 3. Live thrown splash potion via `tick_thrown_potions` (`ThrownPotion`).
            let thrown_immune =
                summon_guarded(&mut game, "minecraft:pig", 4.5, 4.5, "test:splash_guard");
            let thrown_hit =
                summon_guarded(&mut game, "minecraft:pig", 4.5, 4.5, "test:cloud_guard");
            game.potions.thrown.push(ThrownPotion {
                id: 0,
                stack: harming.clone(),
                previous: Vec3::new(4.5, 64.1, 4.5),
                position: Vec3::new(4.5, 64.1, 4.5),
                velocity: Vec3::new(0.0, -0.5, 0.0),
                age: 0,
                owner: None,
                thrower: None,
                guest: None,
                left_owner: true,
                last_deflected_by: None,
                random: LegacyRandom::new(1),
            });
            game.tick_thrown_potions();
            let immune_pig = game.mobs.iter().find(|m| m.id == thrown_immune).unwrap();
            let hit_pig = game.mobs.iter().find(|m| m.id == thrown_hit).unwrap();
            assert_eq!(
                (immune_pig.health, immune_pig.hurt_time),
                (10.0, 0),
                "splash_guard blocks live ThrownPotion splash damage"
            );
            assert!(
                hit_pig.health < 10.0 && hit_pig.hurt_time > 0,
                "mismatched cloud_guard still takes live ThrownPotion splash damage"
            );
            game.mobs.clear();

            // 4. Live water splash on water-sensitive mob (`affect_entities_around`).
            let water_immune =
                summon_guarded(&mut game, "minecraft:blaze", 6.5, 2.5, "test:splash_guard");
            let water_hit =
                summon_guarded(&mut game, "minecraft:blaze", 6.5, 2.5, "test:cloud_guard");
            let water = ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:water");
            let mut water_proj = thrown_projectile(water, Vec3::new(6.5, 64.1, 2.5), true);
            water_proj.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(water_proj);
            game.tick_projectiles().unwrap();
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == water_immune)
                    .unwrap()
                    .health,
                20.0,
                "splash_guard blocks live water splash damage on a blaze"
            );
            assert_eq!(
                game.mobs.iter().find(|m| m.id == water_hit).unwrap().health,
                19.0,
                "mismatched cloud_guard takes 1.0 water splash damage on a blaze"
            );
            game.mobs.clear();

            // 5. Live player worn-immunity for splash potion and lingering cloud impacts.
            game.set_game_mode(sunrise_client::command::args::GameMode::Survival);
            game.player.as_mut().unwrap().position = Vec3::new(2.5, 64.0, 4.5);
            game.armor[3] = Some(boots("test:splash_guard"));
            let mut p_splash = thrown_projectile(harming.clone(), Vec3::new(2.5, 64.1, 4.5), false);
            p_splash.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(p_splash);
            game.tick_projectiles().unwrap();
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                20.0,
                "player wearing splash_guard is immune to live splash potion damage"
            );
            game.armor[3] = Some(boots("test:cloud_guard"));
            let mut p_splash_hit = thrown_projectile(harming, Vec3::new(2.5, 64.1, 4.5), false);
            p_splash_hit.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(p_splash_hit);
            game.tick_projectiles().unwrap();
            assert!(
                game.player.as_ref().unwrap().health.health() < 20.0,
                "player wearing mismatched cloud_guard takes live splash potion damage"
            );
            // Reset player health and verify live lingering cloud on the player too.
            let _ = game.player.as_mut().unwrap().set_health(20.0, false);
            game.player.as_mut().unwrap().health.invulnerable_time = 0;
            game.armor[3] = Some(boots("test:cloud_guard"));
            let mut p_cloud = thrown_projectile(lingering, Vec3::new(2.5, 64.1, 4.5), false);
            p_cloud.velocity = Vec3::new(0.0, -0.5, 0.0);
            game.add_projectile(p_cloud);
            game.tick_projectiles().unwrap();
            for _ in 0..11 {
                game.tick_potion_clouds();
            }
            assert_eq!(
                game.player.as_ref().unwrap().health.health(),
                20.0,
                "player wearing cloud_guard is immune to live lingering cloud damage"
            );
            game.armor[3] = Some(boots("test:splash_guard"));
            for _ in 0..21 {
                game.tick_potion_clouds();
            }
            assert!(
                game.player.as_ref().unwrap().health.health() < 20.0,
                "player wearing mismatched splash_guard takes live lingering cloud damage"
            );
        },
    );
}
