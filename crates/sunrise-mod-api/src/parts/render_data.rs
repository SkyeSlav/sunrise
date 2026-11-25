//! Startup registration of typed per-view render data keys (`RenderDataKey<T>`).
use std::any::TypeId;

use super::*;

impl RenderDataRegistrar for PartRegistrar {
    fn register_render_data_key_raw(
        &mut self,
        id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
    ) {
        match self.type_identities.register(
            TypeIdentityDomain::RenderData,
            id,
            type_id,
            type_name,
            self.mod_id,
        ) {
            Ok(_) => {}
            Err(TypeIdentityError::InvalidName) => {
                self.refused.push(format!(
                    "mod {} registered render data key {id:?}, which is not a namespaced name",
                    self.mod_id
                ));
            }
            Err(TypeIdentityError::Conflict {
                existing_type_name,
                existing_owner,
            }) => {
                self.refused.push(format!(
                    "render data key {id} is registered with two types, {existing_type_name} by {existing_owner} and {type_name} by {}",
                    self.mod_id
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext, RenderDataKey};

    struct AlphaState(u32);
    struct BetaState(String);

    const SHARED_ALPHA: RenderDataKey<AlphaState> = RenderDataKey::new("shared:frame_state");
    const CONFLICTING_BETA: RenderDataKey<BetaState> = RenderDataKey::new("shared:frame_state");
    const BAD_KEY: RenderDataKey<AlphaState> = RenderDataKey::new("not_namespaced");

    struct ModAlpha;
    #[mod_api::__subscriptions]
    impl Hooks for ModAlpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_render_data_key(&SHARED_ALPHA);
        }
    }

    struct ModAlphaPeer;
    #[mod_api::__subscriptions]
    impl Hooks for ModAlphaPeer {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_render_data_key(&SHARED_ALPHA);
        }
    }

    struct ModBetaConflict;
    #[mod_api::__subscriptions]
    impl Hooks for ModBetaConflict {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_render_data_key(&CONFLICTING_BETA);
        }
    }

    struct ModBadKey;
    #[mod_api::__subscriptions]
    impl Hooks for ModBadKey {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_render_data_key(&BAD_KEY);
        }
    }

    struct ModRefusedAlpha;
    #[mod_api::__subscriptions]
    impl Hooks for ModRefusedAlpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_render_data_key(&SHARED_ALPHA);
            r.refuse("deliberate refusal");
        }
    }

    #[test]
    fn two_mods_sharing_render_data_key_with_same_type_succeed() {
        crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("alpha", Box::new(ModAlpha)),
            ModEntry::new("peer", Box::new(ModAlphaPeer)),
        ])
        .unwrap();
    }

    #[test]
    fn conflicting_render_data_key_types_are_refused_naming_both_mods() {
        let errors = crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("alpha", Box::new(ModAlpha)),
            ModEntry::new("beta", Box::new(ModBetaConflict)),
        ])
        .unwrap_err();
        assert!(
            errors.iter().any(|e| {
                e.contains("shared:frame_state")
                    && e.contains("alpha")
                    && e.contains("beta")
                    && e.contains("AlphaState")
                    && e.contains("BetaState")
            }),
            "expected collision refusal naming both mods and both types, got: {errors:?}"
        );
    }

    #[test]
    fn non_namespaced_render_data_key_is_refused() {
        let errors = crate::parts::tests::install_parts_fixture(&[ModEntry::new(
            "badmod",
            Box::new(ModBadKey),
        )])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("badmod") && e.contains("not_namespaced")),
            "expected non-namespaced refusal, got: {errors:?}"
        );
    }

    #[test]
    fn refused_mod_rolls_back_type_identity_entry() {
        let errors = crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("alpha", Box::new(ModRefusedAlpha)),
            ModEntry::new("beta", Box::new(ModBetaConflict)),
        ])
        .unwrap_err();
        assert!(
            !errors.iter().any(|e| e.contains("two types")),
            "rolled-back registration must not leave a stale identity entry: {errors:?}"
        );
    }
}
