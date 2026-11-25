//! Daily tests for `LevelRef::attachments` / `LevelRef::attachments_mut` on parked server
//! levels (persistence, transaction rollback) and `block_changed` on parked-level writes (#5577).

use crate::game::Dim;
use crate::game::native_entity_refs::tests::{fixture_root, with_game};
use mod_api::{
    AttachmentCodec, AttachmentDescriptor, AttachmentError, AttachmentKey, BlockChangedEvent,
    BlockUpdateFlags, Compound, Holder, Hooks, HostTransaction, LevelDimension, LevelHost,
    ModEntry, Registrar, Tag,
};
use std::sync::{Arc, Mutex};
use sunrise_client::chunk::Chunk;

fn counter_descriptor() -> AttachmentDescriptor<i32> {
    AttachmentDescriptor::new("level_attach_test:counter", || 0i32).serialize(AttachmentCodec {
        encode: |value| Ok(Compound::from([("n".into(), Tag::Int(*value))])),
        decode: |data| match data.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    })
}

struct CounterMod(Arc<Mutex<Option<AttachmentKey<i32>>>>);
#[mod_api::__subscriptions]
impl Hooks for CounterMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.register_attachment_type(counter_descriptor()).unwrap();
        *self.0.lock().unwrap() = Some(key);
    }
}

#[test]
#[ignore = "daily"]
fn parked_level_attachments_persist_across_tick_and_reopen() {
    let key_slot = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        "level_attach_test",
        Box::new(CounterMod(key_slot.clone())),
    )]
    .into();
    let root = fixture_root();
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world");

    mod_platform::testing::with_mods(mods, || {
        {
            let mut resources = None;
            let mut game =
                crate::game::levels::tests::open_game(&mut resources, &root, home.path(), &world);
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(game.dim, Dim::Overworld);

            let key = key_slot.lock().unwrap().clone().unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();

            // Holder::Level through the host still names the level in place (Overworld).
            key.set(&mut game, Holder::Level, 11).unwrap();
            nether.attachments_mut(&mut game).set(&key, 42).unwrap();
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(overworld.attachments(&game).get(&key, |v| *v), Ok(11));
            assert_eq!(key.get(&mut game, Holder::Level, |v| *v), Ok(11));
            assert_eq!(
                nether.attachments(&game).get_existing(&key, |v| v.copied()),
                Ok(Some(42))
            );

            // Value survives a Nether tick and returning to Overworld.
            game.enter_level(Dim::Nether).unwrap();
            game.tick_world(true).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(
                nether.attachments(&game).get_existing(&key, |v| v.copied()),
                Ok(Some(42))
            );

            game.save_all_levels(true).unwrap();
        }

        // Reopen the world from disk and verify the parked Nether's attachment persisted.
        let mut resources = None;
        let mut game =
            crate::game::levels::tests::open_game(&mut resources, &root, home.path(), &world);
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(game.dim, Dim::Overworld);

        let key = key_slot.lock().unwrap().clone().unwrap();
        let nether = game.level("minecraft:the_nether").unwrap();
        assert_eq!(
            nether.attachments(&game).get_existing(&key, |v| v.copied()),
            Ok(Some(42))
        );
    });
}

#[test]
#[ignore = "daily"]
fn parked_level_attachment_write_rolls_back_on_transaction_abort() {
    let key_slot = Arc::new(Mutex::new(None));
    let mods = vec![ModEntry::new(
        "level_attach_test",
        Box::new(CounterMod(key_slot.clone())),
    )]
    .into();

    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(game.dim, Dim::Overworld);

            let key = key_slot.lock().unwrap().clone().unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();

            nether.attachments_mut(game).set(&key, 10).unwrap();
            assert_eq!(
                nether.attachments(game).get_existing(&key, |v| v.copied()),
                Ok(Some(10))
            );

            // Writing to parked Nether inside a rolled-back Overworld transaction restores 10.
            let aborted = HostTransaction::transaction(game, |host, _| {
                nether.attachments_mut(host).set(&key, 99).unwrap();
                assert_eq!(
                    nether
                        .attachments(&*host)
                        .get_existing(&key, |v| v.copied()),
                    Ok(Some(99))
                );
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(
                nether
                    .attachments(&*game)
                    .get_existing(&key, |v| v.copied()),
                Ok(Some(10))
            );

            // Nested abort restores the outer transaction's value; outer commit keeps it.
            let committed = HostTransaction::transaction(game, |host, _| {
                nether.attachments_mut(host).set(&key, 20).unwrap();
                let inner = HostTransaction::transaction(host, |host, _| {
                    nether.attachments_mut(host).set(&key, 30).unwrap();
                    Err::<(), ()>(())
                });
                assert!(inner.is_err());
                assert_eq!(
                    nether
                        .attachments(&*host)
                        .get_existing(&key, |v| v.copied()),
                    Ok(Some(20))
                );
                Ok::<(), ()>(())
            });
            assert!(committed.is_ok());
            assert_eq!(
                nether
                    .attachments(&*game)
                    .get_existing(&key, |v| v.copied()),
                Ok(Some(20))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn parked_level_block_write_fires_block_changed_naming_that_level() {
    type Heard = (LevelDimension, [i32; 3], BlockUpdateFlags);

    struct BlockChangedObserver(Arc<Mutex<Vec<Heard>>>);
    #[mod_api::__subscriptions]
    impl Hooks for BlockChangedObserver {
        fn block_changed(&self, _: &dyn LevelHost, event: &mut BlockChangedEvent) {
            self.0.lock().unwrap().push((
                event.dimension,
                [event.pos.x, event.pos.y, event.pos.z],
                event.flags,
            ));
        }
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "parked_block_changed",
        Box::new(BlockChangedObserver(heard.clone())),
    )]
    .into();

    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.enter_level(Dim::Nether).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let oak_log = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_log")
                .unwrap();
            game.world.set(4, 64, 4, oak_log).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(game.dim, Dim::Overworld);
            heard.lock().unwrap().clear();

            let nether = game.level("minecraft:the_nether").unwrap();
            let flags = BlockUpdateFlags::CLIENTS;
            assert_eq!(
                nether.set_block_property(game, [4, 64, 4], "axis", "x", flags),
                Ok(true)
            );
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(
                heard.lock().unwrap().as_slice(),
                &[(LevelDimension::Nether, [4, 64, 4], flags)]
            );
        });
    });
}
