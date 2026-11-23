/// One name and UUID advertised in a server list response, not a live player handle.
// no wire: a row in the in-process server-list event, not a mod payload.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListedPlayerInfo {
    /// Name shown in the list.
    pub name: String,
    /// UUID advertised for this entry.
    pub uuid: u128,
}

impl ListedPlayerInfo {
    /// Create a row for the advertised sample in a server-list response.
    pub fn new(name: impl Into<String>, uuid: u128) -> Self {
        Self {
            name: name.into(),
            uuid,
        }
    }
}
