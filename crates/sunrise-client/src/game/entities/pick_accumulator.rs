//! The running state of `ProjectileUtil.getEntityHitResult`.
use super::Game;

#[derive(Clone, Copy, Debug)]
pub(super) struct PickCandidate {
    pub id: u32,
    pub distance: f64,
}

pub(super) struct PickAccumulator {
    pub hovered: Option<PickCandidate>,
    nearest: f64,
    candidates: Option<Vec<PickCandidate>>,
}

impl PickAccumulator {
    pub(super) fn new(reach: f64) -> Self {
        Self {
            hovered: None,
            nearest: reach,
            candidates: None,
        }
    }

    pub(super) fn consider(&mut self, game: &Game<'_>, id: u32, distance: f64) -> bool {
        if game.pick_camera_id() == Some(id) {
            return false;
        }
        if let Some(candidates) = &mut self.candidates {
            candidates.push(PickCandidate { id, distance });
            return false;
        }
        self.accept(
            PickCandidate { id, distance },
            game.pick_shares_vehicle(id),
            false,
        )
    }

    /// Candidate geometry is copied while the family lists are borrowed; only a
    /// subscribed hook needs this reusable buffer so its Host can borrow Game.
    pub(super) fn recording(reach: f64, mut candidates: Vec<PickCandidate>) -> Self {
        candidates.clear();
        Self {
            candidates: Some(candidates),
            ..Self::new(reach)
        }
    }

    pub(super) fn dispatch(&mut self, game: &mut Game<'_>) -> Vec<PickCandidate> {
        let candidates = self.candidates.take().unwrap_or_default();
        let camera = game.pick_camera_id();
        for &candidate in &candidates {
            if game.pick_camera_id() == Some(candidate.id) {
                continue;
            }
            let same_vehicle = game.pick_shares_vehicle(candidate.id);
            let mut allowed = false;
            if candidate.distance != 0.0
                && (candidate.distance < self.nearest || self.nearest == 0.0)
                && same_vehicle
                && let Some(epoch) = game.native_lifecycle.epoch()
            {
                let entity = mod_api::EntityRef::new(epoch, candidate.id);
                game.native_can_rider_interact(mod_api::__private::CanRiderInteractCall {
                    entity,
                    allowed: &mut allowed,
                });
                if game.pick_camera_id() != camera {
                    // The captured ray and vehicle decision no longer belong
                    // to this camera. Refuse the pick, never recast it.
                    self.hovered = None;
                    break;
                }
            }
            self.accept(candidate, same_vehicle, allowed);
        }
        candidates
    }

    fn accept(&mut self, candidate: PickCandidate, same_vehicle: bool, allowed: bool) -> bool {
        let distance = candidate.distance;
        if distance == 0.0 {
            if self.nearest < 0.0 {
                return false;
            }
            self.nearest = 0.0;
        } else if distance < self.nearest || self.nearest == 0.0 {
            if same_vehicle && !allowed {
                if self.nearest != 0.0 {
                    return false;
                }
            } else {
                self.nearest = distance;
            }
        } else {
            return false;
        }
        self.hovered = Some(candidate);
        true
    }
}

impl Game<'_> {
    pub(super) fn pick_camera_id(&self) -> Option<u32> {
        self.remote_camera_id()
            .or_else(|| self.remote.as_ref().map(|r| r.entity_id as u32))
    }

    fn pick_vehicle_root(&self, id: u32) -> Option<u32> {
        let mut root = id;
        let limit = self
            .remote
            .as_ref()
            .map_or(self.mobs.len(), |r| r.passenger_vehicles.len());
        for _ in 0..=limit + 1 {
            let vehicle =
                if self.pick_camera_id() == Some(root) && self.remote_camera_id().is_none() {
                    self.riding
                } else if let Some(remote) = &self.remote {
                    remote.passenger_vehicles.get(&root).copied()
                } else {
                    self.vehicle_of_any(root)
                };
            let Some(vehicle) = vehicle else {
                return Some(root);
            };
            root = vehicle;
        }
        None
    }

    pub(super) fn pick_shares_vehicle(&self, id: u32) -> bool {
        let camera_root = self
            .pick_camera_id()
            .and_then(|camera| self.pick_vehicle_root(camera))
            .or_else(|| {
                self.riding
                    .and_then(|vehicle| self.pick_vehicle_root(vehicle))
            });
        camera_root.is_some() && self.pick_vehicle_root(id) == camera_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_vehicle_uses_the_running_nearest_not_the_candidates_distance() {
        let mut pick = PickAccumulator::new(5.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 2,
                distance: 1.0
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 1,
                distance: 0.0
            },
            true,
            false
        ));
        assert!(pick.accept(
            PickCandidate {
                id: 2,
                distance: 3.0
            },
            true,
            false
        ));
        assert_eq!(pick.hovered.unwrap().id, 2);
        assert_eq!(pick.nearest, 0.0);
        assert!(pick.accept(
            PickCandidate {
                id: 3,
                distance: 4.0
            },
            false,
            false
        ));
        assert_eq!(pick.nearest, 4.0);
        assert!(!pick.accept(
            PickCandidate {
                id: 4,
                distance: 4.5
            },
            false,
            false
        ));
    }
}
