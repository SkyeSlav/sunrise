use super::*;

impl Game<'_> {
    /// `IItemExtension.shouldCauseReequipAnimation`: does `hand` lower and
    /// rise again to show `stack` in place of `old`? `reequip` arrives with
    /// vanilla's answer; the parts of `old`'s item may set it. Client-side.
    #[inline(never)]
    pub(in crate::game) fn native_should_cause_reequip_animation(
        &self,
        stack: &ItemStack,
        old: &ItemStack,
        hand: Hand,
        slot: mod_api::EquipmentSlot,
        slot_changed: bool,
        reequip: &mut bool,
    ) {
        self.fire_should_cause_reequip_animation(stack, old, hand, slot, slot_changed, reequip);
    }
}
