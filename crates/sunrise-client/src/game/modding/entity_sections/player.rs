//! File and tell each player move before `Entity.move` resumes.

use super::{EntitySections, Game, PlayerMoveState, Scope, SectionWriter, tell_live};
use crate::game::native_lifecycle::NativeLifecycle;
use mod_api::{EntityRef, PlayerRef};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use sunrise_client::motion::{
    PlayerMotion,
    section_writes::{PositionObserver, PositionObserverScope},
};
use sunrise_core::geometry::Vec3;

pub(in crate::game) struct PlayerSectionScope {
    scope: Scope,
    id: u32,
    body_id: i32,
    local_player: Option<PlayerRef>,
    server: bool,
    atlas: Option<(Arc<sunrise_client::assets::RenderAssets>, u32)>,
}

#[derive(Default)]
pub(in crate::game) struct PlayerSectionObserver {
    active: RefCell<Option<PlayerSectionScope>>,
    sections: RefCell<EntitySections>,
}

impl PositionObserver for PlayerSectionObserver {
    fn position_written(&self, player: &PlayerMotion, from: Vec3) {
        let active = self.active.borrow();
        let Some(active) = active
            .as_ref()
            .filter(|active| active.body_id == player.entity_id)
        else {
            return;
        };
        let to = player.position.components();
        self.sections
            .borrow_mut()
            .moved(active.scope, active.id, Some(from.components()), to);
        tell_live(
            &self.sections,
            SectionWriter {
                entity: EntityRef::new(active.scope.epoch, active.id),
                position: to,
                player: Some(PlayerMoveState {
                    local_player: active.local_player,
                    on_ground: player.ground.on_ground,
                    velocity: player.velocity.components(),
                }),
            },
            active.server,
            active
                .atlas
                .as_ref()
                .map(|(atlas, split)| (atlas.as_ref(), *split)),
        );
    }
}

/// Return the table even if the move fails after committing a position.
pub(in crate::game) struct PlayerSectionPass<'a> {
    _motion: PositionObserverScope,
    observer: &'a PlayerSectionObserver,
    sections: &'a mut EntitySections,
}

impl Drop for PlayerSectionPass<'_> {
    fn drop(&mut self) {
        self.observer.active.take();
        std::mem::swap(self.sections, &mut self.observer.sections.borrow_mut());
    }
}

impl PlayerSectionScope {
    pub(in crate::game) fn begin(self, lifecycle: &mut NativeLifecycle) -> PlayerSectionPass<'_> {
        let observer = lifecycle
            .player_section_observer
            .get_or_insert_with(Default::default);
        observer.active.replace(Some(self));
        let sections = &mut lifecycle.entity_sections;
        std::mem::swap(sections, &mut observer.sections.borrow_mut());
        PlayerSectionPass {
            _motion: PositionObserverScope::install(Rc::clone(observer) as Rc<dyn PositionObserver>),
            observer,
            sections,
        }
    }
}

impl Game<'_> {
    pub(in crate::game) fn player_section_scope(&self) -> Option<PlayerSectionScope> {
        let scope = self.entity_section_scope()?;
        let id = u32::try_from(self.own_entity_id())
            .ok()
            .filter(|id| *id != 0)?;
        let body_id = self.player.as_ref()?.entity_id;
        let server = self.is_logical_server();
        let atlas = (!server).then(|| {
            let (atlas, split) = self.published_block_atlas();
            (Arc::clone(atlas), split)
        });
        Some(PlayerSectionScope {
            scope,
            id,
            body_id,
            local_player: self.local_player(),
            server,
            atlas,
        })
    }
}
