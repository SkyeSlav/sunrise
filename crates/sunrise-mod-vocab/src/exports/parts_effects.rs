//! Effect Part registration and paired engine dispatch.
pub use parts::MobEffectPartError;
mod __private {
    pub use crate::parts::{
        dispatch_mob_effect_tick, has_effect_tick_parts, validate_effect_part_owners,
    };
}
