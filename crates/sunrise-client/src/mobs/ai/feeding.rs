//! `Animal.mobInteract` food acceptance.
use super::{Mob, Species, can_age_up};

/// Adult breeding eligibility at `Animal.mobInteract` after the food and
/// species-specific interaction have been selected.
pub fn can_enter_love_mode(m: &Mob) -> bool {
    m.extra.age == 0
        && m.extra.in_love <= 0
        && match m.species() {
            Species::Turtle => !crate::mobs::turtle::has_egg(m),
            Species::CamelHusk | Species::HappyGhast | Species::ZombieHorse => false,
            _ => !crate::mobs::piglin::refuses_love(m),
        }
}

/// Whether `Animal.mobInteract` takes the animal's food: an adult entering
/// love or a baby growing. Tamed wolves and cats can instead sit or stand.
pub fn takes_food(m: &Mob) -> bool {
    can_enter_love_mode(m) || can_age_up(m)
}
