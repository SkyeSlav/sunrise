//! Load-owned type bindings for the game's default providers.
use super::*;

impl PartRegistrar {
    fn default_block<P>(
        &mut self,
        block: &crate::BlockKey,
        declare: impl FnOnce(&str) -> Result<P, String>,
        table: fn(&mut Parts) -> &mut HashMap<Box<str>, Vec<(usize, P)>>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .blocks
            .id(mod_vocab::__private::parts_installation::block_key_slot(
                *block,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {} declared a default provider for an invalid block key",
                self.mod_id
            ));
            return;
        };
        if id.starts_with("minecraft:") {
            self.refused.push(format!("mod {} declared a mod block default provider for vanilla block {id}; use block_capability instead", self.mod_id));
            return;
        }
        let declaration = match declare(&id) {
            Ok(declaration) => declaration,
            Err(reason) => {
                self.refuse(&reason);
                return;
            }
        };
        table(&mut self.parts)
            .entry(id.into())
            .or_default()
            .push((self.mod_index, declaration));
    }
}

impl DefaultCapabilitiesRegistrar for PartRegistrar {
    fn block_container(&mut self, block: &crate::BlockKey, container: crate::ModContainer) {
        self.default_block(
            block,
            |id| {
                // Vanilla ItemStackWithSlot.CODEC saves each Slot as an unsigned byte.
                const MAX_SAVED_SLOTS: usize = u8::MAX as usize + 1;
                let slots = container.slots;
                if slots > MAX_SAVED_SLOTS {
                    return Err(format!(
                        "{id}: container slots ({slots}) exceed the saved limit of {MAX_SAVED_SLOTS}"
                    ));
                }
                Ok(container)
            },
            |parts| &mut parts.default_block_containers,
        );
    }
    fn block_fluids(&mut self, block: &crate::BlockKey, fluids: crate::ModFluidTanks) {
        self.default_block(
            block,
            |_| Ok(fluids),
            |parts| &mut parts.default_block_fluids,
        );
    }
    fn block_energy(&mut self, block: &crate::BlockKey, energy: crate::ModEnergyBuffer) {
        self.default_block(
            block,
            |_| Ok(energy),
            |parts| &mut parts.default_block_energy,
        );
    }
    fn register_item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: Arc<dyn crate::ItemContainerProvider>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .items
            .id(mod_vocab::__private::parts_installation::item_key_slot(
                *item,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {} declared a default provider for an invalid item key",
                self.mod_id
            ));
            return;
        };
        self.parts
            .default_item_containers
            .entry(id.into())
            .or_default()
            .push((self.mod_index, provider));
    }
}
