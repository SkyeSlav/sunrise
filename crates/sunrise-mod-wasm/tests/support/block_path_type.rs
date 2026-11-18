use mod_api::{BlockEntityRef, BlockPathTypeEvent, BlockState, Compound, LevelHost, PathType, Tag};
use std::cell::RefCell;

pub struct World {
    state: BlockState,
    pub asked: RefCell<Vec<[i32; 3]>>,
}

fn chest() -> BlockEntityRef {
    mod_api::__private::game::block_entity_ref(
        mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(1),
            mod_api::DimensionKey::OVERWORLD,
        ),
        [0, 64, 1],
        7,
    )
}

impl LevelHost for World {
    fn block_entity(&self, pos: [i32; 3]) -> Option<BlockEntityRef> {
        self.asked.borrow_mut().push(pos);
        (pos == [0, 64, 1]).then_some(chest())
    }
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        (entity == chest()).then(|| Compound::from([("count".into(), Tag::Int(3))]))
    }
}

impl World {
    pub fn new() -> Self {
        let properties = std::collections::BTreeMap::new();
        mod_api::__private::resolve_block_keys(["minecraft:stone"], |_| {
            sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                .map_err(|error| format!("{error:?}"))
        })
        .unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        let key = mod_api::__private::part_tests::block_key("minecraft:stone");
        Self {
            state: BlockState::of(metadata, key, &[]).unwrap(),
            asked: RefCell::new(Vec::new()),
        }
    }

    pub fn check(&self, part: &dyn mod_api::BlockPart) {
        self.asked.borrow_mut().clear();
        let mut event = BlockPathTypeEvent {
            pos: mod_api::BlockPos::new(1, 64, 1),
            state: self.state,
        };
        assert_eq!(
            part.block_path_type(self, &mut event),
            Some(PathType::Damaging)
        );
        assert_eq!(*self.asked.borrow(), [[0, 64, 1]]);
    }

    /// The hook through the installed parts: a compiled guest part answers and
    /// reads the neighbouring block entity through this same level.
    pub fn dispatch(&self) {
        self.asked.borrow_mut().clear();
        let mut out = None;
        mod_api::dispatch_block_path_type(
            self,
            mod_api::BlockPos::new(1, 64, 1),
            self.state,
            &mut out,
        );
        assert_eq!(out, Some(PathType::Damaging));
        assert_eq!(*self.asked.borrow(), [[0, 64, 1]]);
    }
}
