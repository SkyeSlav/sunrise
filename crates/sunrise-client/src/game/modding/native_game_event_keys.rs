//! Startup game-event registry reads.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn game_event_key(&self, id: &str) -> Option<mod_api::GameEventKey> {
        mod_api::__private::game_event_key(id)
    }
    pub(in crate::game) fn game_event_key_id(
        &self,
        key: mod_api::GameEventKey,
    ) -> Result<mod_api::RegistryName, mod_api::GameEventKeyError> {
        mod_api::__private::game_event_key_id(key).ok_or(mod_api::GameEventKeyError::InvalidKey)
    }
    pub(in crate::game) fn profession_key_id(&self, key: &mod_api::ProfessionKey) -> Option<&str> {
        let id = key.0.to_string();
        if let Some(remote) = &self.remote {
            return remote.professions.canonical_id(&id);
        }
        self.stations().professions.canonical_id(&id)
    }
}
