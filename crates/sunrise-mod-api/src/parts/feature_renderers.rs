//! Startup registration of typed feature renderer keys (`FeatureRendererType<S>`).
use std::any::TypeId;

use super::*;

impl FeatureRendererRegistrar for PartRegistrar {
    fn register_feature_renderer_type_raw(
        &mut self,
        id: &'static str,
        type_id: TypeId,
        type_name: &'static str,
    ) {
        match self.type_identities.register(
            TypeIdentityDomain::FeatureRenderer,
            id,
            type_id,
            type_name,
            self.mod_id,
        ) {
            Ok(_) => {}
            Err(TypeIdentityError::InvalidName) => {
                self.refused.push(format!(
                    "mod {} registered feature renderer type {id:?}, which is not a namespaced name",
                    self.mod_id
                ));
            }
            Err(TypeIdentityError::Conflict {
                existing_type_name,
                existing_owner,
            }) => {
                self.refused.push(format!(
                    "feature renderer type {id} is registered with two types, {existing_type_name} by {existing_owner} and {type_name} by {}",
                    self.mod_id
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{FeatureRendererType, Hooks, ModEntry, Registrar, RegistrationContext};

    struct AlphaNode(u32);
    struct BetaNode(String);

    const SHARED_ALPHA: FeatureRendererType<AlphaNode> =
        FeatureRendererType::new("shared:beam_feature");
    const CONFLICTING_BETA: FeatureRendererType<BetaNode> =
        FeatureRendererType::new("shared:beam_feature");
    const BAD_TYPE: FeatureRendererType<AlphaNode> = FeatureRendererType::new("not_namespaced");

    struct ModAlpha;
    #[mod_api::__subscriptions]
    impl Hooks for ModAlpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_feature_renderer_type(&SHARED_ALPHA);
        }
    }

    struct ModAlphaPeer;
    #[mod_api::__subscriptions]
    impl Hooks for ModAlphaPeer {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_feature_renderer_type(&SHARED_ALPHA);
        }
    }

    struct ModBetaConflict;
    #[mod_api::__subscriptions]
    impl Hooks for ModBetaConflict {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_feature_renderer_type(&CONFLICTING_BETA);
        }
    }

    struct ModBadType;
    #[mod_api::__subscriptions]
    impl Hooks for ModBadType {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_feature_renderer_type(&BAD_TYPE);
        }
    }

    struct ModRefusedAlpha;
    #[mod_api::__subscriptions]
    impl Hooks for ModRefusedAlpha {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_feature_renderer_type(&SHARED_ALPHA);
            r.refuse("deliberate refusal");
        }
    }

    #[test]
    fn two_mods_sharing_feature_renderer_type_with_same_rust_type_succeed() {
        crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("alpha", Box::new(ModAlpha)),
            ModEntry::new("peer", Box::new(ModAlphaPeer)),
        ])
        .unwrap();
    }

    #[test]
    fn conflicting_feature_renderer_types_are_refused_naming_both_mods() {
        let errors = crate::parts::tests::install_parts_fixture(&[
            ModEntry::new("alpha", Box::new(ModAlpha)),
            ModEntry::new("beta", Box::new(ModBetaConflict)),
        ])
        .unwrap_err();
        assert!(
            errors.iter().any(|e| {
                e.contains("shared:beam_feature")
                    && e.contains("alpha")
                    && e.contains("beta")
                    && e.contains("AlphaNode")
                    && e.contains("BetaNode")
            }),
            "expected collision refusal naming both mods and both types, got: {errors:?}"
        );
    }

    #[test]
    fn non_namespaced_feature_renderer_type_is_refused() {
        let errors = crate::parts::tests::install_parts_fixture(&[ModEntry::new(
            "badmod",
            Box::new(ModBadType),
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
    fn refused_mod_rolls_back_feature_renderer_type_identity_entry() {
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
