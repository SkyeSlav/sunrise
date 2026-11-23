use super::super::*;
use crate::*;
use std::sync::Mutex;

type Trace = Arc<Mutex<Vec<(&'static str, f32)>>>;
#[derive(Clone)]
struct Cadence<const PERIOD: i32> {
    package: &'static str,
    trace: Trace,
}
#[mod_api::sunrise_part]
impl<const PERIOD: i32> MobEffectPart for Cadence<PERIOD> {
    fn should_apply_effect_tick_this_tick(
        &self,
        _: &dyn Host,
        event: &mut ShouldApplyEffectTickThisTickEvent,
    ) {
        assert!(!event.apply);
        event.apply = event.tick_count % PERIOD == 0;
    }
    fn apply_effect_tick(&self, host: &mut dyn ServerHost, event: &mut ApplyEffectTickEvent) {
        let health = event.holder.health(host).unwrap();
        self.trace.lock().unwrap().push((self.package, health));
        assert!(event.holder.heal(host, PERIOD as f32));
        if matches!(
            (self.package, event.amplifier),
            ("owner", 99) | ("other", 100)
        ) {
            event.keep = false;
        }
    }
}
#[mod_api::__subscriptions]
impl<const PERIOD: i32> Hooks for Cadence<PERIOD> {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let effect = r.effect_key("owner:comfort");
        r.mob_effect_part(&effect, self.clone()).unwrap();
    }
}

fn package(id: &str, dependency: Option<DependencyKind>) -> PackageInfo {
    PackageInfo::new(
        id.into(),
        "1".into(),
        PackageKind::Code,
        Side::Both,
        vec![id.into()],
        dependency
            .into_iter()
            .map(|kind| {
                PackageDependency::new(
                    "owner".into(),
                    "*".into(),
                    kind,
                    DependencyOrder::After,
                    Side::Both,
                    None,
                    None,
                )
            })
            .collect(),
    )
}

#[test]
fn optional_effect_parts_skip_only_when_the_package_is_absent() {
    for present in [false, true] {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "behaviour",
            Box::new(Cadence::<3> {
                package: "behaviour",
                trace,
            }),
        )]
        .into();
        let mut packages = vec![package("behaviour", Some(DependencyKind::Optional))];
        if present {
            packages.push(package("owner", None));
        }
        crate::load_state::with_test_package_inventory(packages, || {
            crate::testing::with_mods(mods.clone(), || {
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                let ids = if present {
                    vec!["owner:comfort"]
                } else {
                    vec![]
                };
                mod_vocab::__private::resolve_effects(ids, |_| present.then_some(0), |_| true)
                    .unwrap();
                mod_vocab::__private::validate_effect_part_owners(
                    |_| present.then_some("owner"),
                    |_| true,
                )
                .unwrap();
                assert_eq!(
                    mod_vocab::__private::effect_key_by_index(0).is_some(),
                    present
                );
            });
        });
    }
}

/// An optional provider that is listed when the Parts are installed but whose content layer was
/// refused (an opened, refused package) is not admitted: the dependent's Part is dropped. A provider
/// that is admitted and lacks the effect keeps the unknown-effect refusal.
#[test]
fn an_optional_effect_part_of_a_refused_provider_is_dropped_and_an_admitted_one_refused() {
    for admitted in [false, true] {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "behaviour",
            Box::new(Cadence::<3> {
                package: "behaviour",
                trace,
            }),
        )]
        .into();
        let packages = vec![
            package("behaviour", Some(DependencyKind::Optional)),
            package("owner", None),
        ];
        crate::load_state::with_test_package_inventory(packages, || {
            crate::testing::with_mods(mods.clone(), || {
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                // The provider never registered the effect.
                let resolved = mod_vocab::__private::resolve_effects(
                    [],
                    |_| None,
                    |package| package == "owner" && admitted,
                );
                if admitted {
                    assert_eq!(
                        resolved.unwrap_err(),
                        ["mod behaviour requested unknown effect owner:comfort"]
                    );
                } else {
                    resolved.unwrap();
                    mod_vocab::__private::validate_effect_part_owners(|_| None, |_| false).unwrap();
                }
            });
        });
    }
}

#[test]
fn unknown_required_effects_keep_the_key_resolution_diagnostic() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "behaviour",
        Box::new(Cadence::<3> {
            package: "behaviour",
            trace,
        }),
    )]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![package("behaviour", Some(DependencyKind::Required))],
        || {
            crate::testing::with_mods(mods.clone(), || {
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                assert_eq!(
                    mod_vocab::__private::resolve_effects([], |_| None, |_| true).unwrap_err(),
                    ["mod behaviour requested unknown effect owner:comfort"]
                );
            });
        },
    );
}

#[test]
fn registration_refuses_vanilla_and_duplicate_effect_parts() {
    struct Registration;
    #[mod_api::__subscriptions]
    impl Hooks for Registration {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let part = || Cadence::<2> {
                package: "owner",
                trace: Arc::new(Mutex::new(Vec::new())),
            };
            let vanilla = r.effect_key("minecraft:poison");
            assert_eq!(
                r.mob_effect_part(&vanilla, part()),
                Err(MobEffectPartError::VanillaEffect { effect: vanilla })
            );
            let custom = r.effect_key("minecraft:comfort");
            r.mob_effect_part(&custom, part()).unwrap();
            assert_eq!(
                r.mob_effect_part(&custom, part()),
                Err(MobEffectPartError::Duplicate {
                    effect: custom,
                    part: std::any::type_name::<Cadence<2>>().into(),
                })
            );
        }
    }
    let errors = super::super::shared::tests::with_builtin_part_registry(
        "minecraft:mob_effect",
        "minecraft:poison",
        || {
            install_parts(
                &[ModEntry::new("owner", Box::new(Registration))],
                &crate::parts::tests::rule_schema(),
                |_| false,
            )
            .unwrap_err()
        },
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("minecraft:comfort"))
    );
}

#[test]
fn registration_refuses_malformed_effect_key_distinctly_from_duplicate() {
    struct Registration;
    #[mod_api::__subscriptions]
    impl Hooks for Registration {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let part = Cadence::<2> {
                package: "owner",
                trace: Arc::new(Mutex::new(Vec::new())),
            };
            let invalid = r.effect_key("Bad Id");
            assert_eq!(
                r.mob_effect_part(&invalid, part),
                Err(MobEffectPartError::InvalidKey { effect: invalid })
            );
        }
    }
    let errors = install_parts(
        &[ModEntry::new("owner", Box::new(Registration))],
        &crate::parts::tests::rule_schema(),
        |_| false,
    )
    .unwrap_err();
    assert_eq!(
        errors,
        [
            "mod owner: invalid registry key \"minecraft:Bad Id\"",
            &format!(
                "mod owner attached {} to an invalid effect key",
                std::any::type_name::<Cadence<2>>()
            ),
        ]
    );
}

struct World {
    health: f32,
}
impl crate::prelude::LivingBackend for World {
    fn living_health(&self, _: LivingRef) -> Option<f32> {
        Some(self.health)
    }
}
impl crate::prelude::LivingBackendMut for World {
    fn living_heal(&mut self, _: LivingRef, amount: f32) -> bool {
        self.health += amount;
        true
    }
}
impl crate::prelude::LivingClientBackend for World {}
impl Backend for World {}
crate::impl_backend_areas!(World, except(living));
impl BackendMut for World {}
crate::impl_backend_mut_areas!(World, except(living));
impl Host for World {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(mod_vocab::__private::parts_installation::backend_ref(self))
    }
}
crate::impl_host_areas!(World);
impl ServerHost for World {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(mod_vocab::__private::parts_installation::backend_mut_ref(
            self,
        ))
    }
}
crate::impl_server_host_areas!(World);

fn install(mods: &[ModEntry]) -> EffectKey {
    install_parts(mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mod_vocab::__private::resolve_effects(["owner:comfort"], |_| Some(0), |_| true).unwrap();
    mod_vocab::__private::effect_key_by_index(0).unwrap()
}

#[test]
fn effect_owner_runs_first_even_when_the_dependent_dispatches_first() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        // The package resolver can put dependents first (order = "before").
        ModEntry::new(
            "other",
            Box::new(Cadence::<6> {
                package: "other",
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "behaviour",
            Box::new(Cadence::<3> {
                package: "behaviour",
                trace: trace.clone(),
            }),
        ),
        ModEntry::new(
            "owner",
            Box::new(Cadence::<2> {
                package: "owner",
                trace: trace.clone(),
            }),
        ),
    ]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![
            package("owner", None),
            package("behaviour", Some(DependencyKind::Required)),
            package("other", Some(DependencyKind::Required)),
        ],
        || {
            crate::testing::with_mods(mods.clone(), || {
                let effect = install(&mods);
                mod_vocab::__private::validate_effect_part_owners(|_| Some("owner"), |_| true)
                    .unwrap();
                let level = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::OVERWORLD);
                let holder = LivingRef::new(EntityRef::new(WorldEpoch::new(1), 7));
                for (ticks, amplifier, expected_health, expected_trace, expected_keep) in [
                    (1, 0, 10.0, vec![], true),
                    (2, 0, 12.0, vec![("owner", 10.0)], true),
                    (3, 0, 13.0, vec![("behaviour", 10.0)], true),
                    (
                        6,
                        0,
                        21.0,
                        vec![("owner", 10.0), ("other", 12.0), ("behaviour", 18.0)],
                        true,
                    ),
                    (6, 99, 12.0, vec![("owner", 10.0)], false),
                    (6, 100, 18.0, vec![("owner", 10.0), ("other", 12.0)], false),
                ] {
                    trace.lock().unwrap().clear();
                    let mut world = World { health: 10.0 };
                    let keep = mod_vocab::__private::dispatch_mob_effect_tick(
                        &mut world, level, holder, effect, ticks, amplifier,
                    );
                    assert_eq!(world.health, expected_health);
                    assert_eq!(*trace.lock().unwrap(), expected_trace);
                    assert_eq!(keep, expected_keep);
                }
            });
        },
    );
}

#[test]
fn a_removed_dependency_row_cannot_authorize_a_foreign_effect_part() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "behaviour",
        Box::new(Cadence::<3> {
            package: "behaviour",
            trace,
        }),
    )]
    .into();
    crate::load_state::with_test_package_inventory(
        vec![
            package("owner", None),
            package("behaviour", Some(DependencyKind::Required)),
        ],
        || {
            // The manifest still declares owner; the effective graph lost that row.
            crate::load_state::with_test_dependencies(Default::default(), || {
                crate::testing::with_mods(mods.clone(), || {
                    install(&mods);
                    let errors = mod_vocab::__private::validate_effect_part_owners(
                        |_| Some("owner"),
                        |_| true,
                    )
                    .unwrap_err();
                    assert_eq!(
                        errors,
                        [format!(
                            "mod behaviour attached {} to effect owner:comfort, declared by package owner; declare owner as a dependency",
                            std::any::type_name::<Cadence<3>>()
                        )]
                    );
                });
            });
        },
    );
}

#[test]
fn on_effect_added_part_runs_owner_first_and_only_after_validation() {
    struct NoHost;
    impl Host for NoHost {}
    crate::impl_host_areas!(NoHost);
    impl ServerHost for NoHost {}
    crate::impl_server_host_areas!(NoHost);

    #[derive(Clone)]
    // Each Part type can attach once to this effect, even across packages.
    struct AddedRecorder<const OWNER: bool>(Arc<Mutex<Vec<(&'static str, i32)>>>);
    #[mod_api::sunrise_part]
    impl<const OWNER: bool> MobEffectPart for AddedRecorder<OWNER> {
        fn on_effect_added(&self, _: &mut dyn ServerHost, event: &mut OnEffectAddedEvent) {
            let package = if OWNER { "owner" } else { "behaviour" };
            self.0.lock().unwrap().push((package, event.amplifier));
        }
    }
    #[mod_api::__subscriptions]
    impl<const OWNER: bool> Hooks for AddedRecorder<OWNER> {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let effect = r.effect_key("owner:comfort");
            r.mob_effect_part(&effect, self.clone()).unwrap();
        }
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        ModEntry::new("behaviour", Box::new(AddedRecorder::<false>(seen.clone()))),
        ModEntry::new("owner", Box::new(AddedRecorder::<true>(seen.clone()))),
    ]
    .into();
    let living = LivingRef::new(EntityRef::new(WorldEpoch::new(1), 7));
    crate::load_state::with_test_package_inventory(
        vec![
            package("owner", None),
            package("behaviour", Some(DependencyKind::Required)),
        ],
        || {
            crate::testing::with_mods(mods.clone(), || {
                let effect = install(&mods);
                // Before owner validation freezes the table, no MobEffectPart::on_effect_added runs.
                dispatch_on_effect_added(&mut NoHost, effect, living, 1);
                assert!(seen.lock().unwrap().is_empty());

                mod_vocab::__private::validate_effect_part_owners(|_| Some("owner"), |_| true)
                    .unwrap();
                dispatch_on_effect_added(&mut NoHost, effect, living, 2);
                assert_eq!(*seen.lock().unwrap(), vec![("owner", 2), ("behaviour", 2)]);
            });
        },
    );
}
