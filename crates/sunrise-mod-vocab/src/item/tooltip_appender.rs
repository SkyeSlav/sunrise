//! A data component's own tooltip lines (NeoForge's `RegisterTooltipAppendersEvent`): a mod
//! registers an appender for one component with `Registrar::register_tooltip_appender`, and the
//! game asks it for a stack that carries the component and whose tooltip shows it.

use crate::{
    ClientHost, Identifier, ItemStack, Text,
    world::reload_order::{ReloadOrder, ReloadOrderError},
};

/// A constraint in NeoForge's component tooltip appender order.
/// `Before` and `After` may refer to a vanilla or modded appender, including one
/// registered later. They express relative precedence, not adjacency. Among ready
/// appenders, vanilla's sequence takes precedence, then mods' registration order.
/// The `registerAppender(TooltipLocation, ...)` overload is not represented.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TooltipPlacement {
    /// Before the first vanilla component's lines, after the item's own lines.
    BeforeAll,
    /// Before the appender for the named data component.
    Before(String),
    /// After the appender for the named data component.
    After(String),
    /// After the last vanilla component (durability), before the advanced item ID and count.
    AfterAll,
}

/// Shared context for an item's own tooltip and component appenders: player,
/// flags and vanilla `Item.TooltipContext.tickRate()`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TooltipAppendContext {
    /// The local player's UUID, including for nested projectiles where vanilla passes no player.
    pub player: u128,
    /// The local player's entity id.
    pub player_entity_id: i32,
    /// The client level's tick rate, or 20 when no client level is present.
    pub tick_rate: f32,
    /// `TooltipFlag.isAdvanced` (F3+H).
    pub advanced: bool,
    /// `TooltipFlag.isCreative`: a creative tab's own slot.
    pub creative: bool,
    /// A crossbow's loaded projectile, whose details its crossbow lists.
    pub nested: bool,
}

/// The lines a data component adds to its stack's tooltip. Kept by the game from registration on;
/// asked only for a stack that carries the component and whose `tooltip_display` shows it, and
/// only while its mod is enabled. One appender per component, including vanilla's own appenders.
pub trait TooltipAppender: Send + Sync + 'static {
    /// Add lines to `lines` for `stack`, each with a colour as `rgba` packs it (`u32::MAX` keeps
    /// the text's own). `context` is the tooltip's player, flags and client-level tick rate.
    fn append(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        context: &TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    );
}

// A single stock appender sequence drives both graph initialization and the engine's
// dispatch boundaries (even for details that emit no lines on a particular stack).
macro_rules! vanilla_details {
    ($($detail:ident => $path:literal),+ $(,)?) => {
        /// The stock component appender slots in `ItemStack.addDetailsToTooltip` order.
        #[repr(usize)]
        #[derive(Clone, Copy)]
        pub enum VanillaTooltipDetail { $($detail),+ }

        impl VanillaTooltipDetail {
            const ALL: &[Self] = &[$(Self::$detail),+];
            const fn id(self) -> &'static str {
                match self { $(Self::$detail => concat!("minecraft:", $path)),+ }
            }
        }
    };
}

// NeoForge's VanillaDataComponentTooltips.collectVanillaAppenders, keyed by DataComponents.
vanilla_details! {
    TropicalFishPattern => "tropical_fish/pattern",
    Instrument => "instrument",
    MapId => "map_id",
    Bees => "bees",
    ContainerLoot => "container_loot",
    Container => "container",
    BannerPatterns => "banner_patterns",
    PotDecorations => "pot_decorations",
    WrittenBookContent => "written_book_content",
    ChargedProjectiles => "charged_projectiles",
    Fireworks => "fireworks",
    FireworkExplosion => "firework_explosion",
    PotionContents => "potion_contents",
    JukeboxPlayable => "jukebox_playable",
    Trim => "trim",
    StoredEnchantments => "stored_enchantments",
    Enchantments => "enchantments",
    DyedColor => "dyed_color",
    Profile => "profile",
    Lore => "lore",
    SulfurCubeContent => "sulfur_cube_content",
    AttributeModifiers => "attribute_modifiers",
    IntangibleProjectile => "intangible_projectile",
    Unbreakable => "unbreakable",
    SignTextFront => "sign_text_front",
    SignTextBack => "sign_text_back",
    OminousBottleAmplifier => "ominous_bottle_amplifier",
    SuspiciousStewEffects => "suspicious_stew_effects",
    BlockState => "block_state",
    EntityData => "entity_data",
    BlockEntityData => "block_entity_data",
    CanBreak => "can_break",
    CanPlaceOn => "can_place_on",
    Damage => "damage",
}

/// Freeze the appenders by `ItemTooltipHandler` and `CommonHooks.sortGraphChecked`.
/// Each result is (number of preceding stock nodes, mod registration index).
pub fn sort_tooltip_appenders<'a>(
    registrations: impl IntoIterator<Item = (&'a Identifier, &'a TooltipPlacement, &'static str)>,
) -> Result<Vec<(usize, usize)>, String> {
    let mut graph = ReloadOrder::default();
    let vanilla: Vec<_> = VanillaTooltipDetail::ALL
        .iter()
        .map(|detail| Identifier::parse_namespaced(detail.id()).expect("stock component ID"))
        .collect();
    for id in &vanilla {
        graph
            .add_listener(id.clone(), None)
            .expect("unique stock appender");
    }
    for pair in vanilla.windows(2) {
        graph
            .add_dependency(pair[0].clone(), pair[1].clone())
            .expect("stock appender");
    }
    let mut pending = Vec::new();
    for (index, (id, placement, mod_id)) in registrations.into_iter().enumerate() {
        graph
            .add_listener(id.clone(), Some(index))
            .map_err(|_| format!("mod {mod_id}: duplicate tooltip appender for {id}"))?;
        pending.push((id, placement, mod_id));
    }
    for (id, placement, mod_id) in pending {
        let missing = |anchor: &str| {
            format!(
                "mod {mod_id}: tooltip appender for {id} has no appender to anchor to at {anchor}"
            )
        };
        let (anchor, before) = match placement {
            TooltipPlacement::BeforeAll => (vanilla[0].clone(), true),
            TooltipPlacement::AfterAll => (vanilla[vanilla.len() - 1].clone(), false),
            TooltipPlacement::Before(anchor) | TooltipPlacement::After(anchor) => (
                Identifier::parse_namespaced(anchor).ok_or_else(|| missing(anchor))?,
                matches!(placement, TooltipPlacement::Before(_)),
            ),
        };
        if *id == anchor {
            return Err(format!(
                "mod {mod_id}: tooltip appender for {id} cannot anchor to itself"
            ));
        }
        let (from, to) = if before { (id, &anchor) } else { (&anchor, id) };
        graph
            .add_dependency(from.clone(), to.clone())
            .map_err(|_| missing(anchor.as_str()))?;
    }
    let mut slot = 0;
    Ok(graph
        .sort(None)
        .map_err(|error| match error {
            ReloadOrderError::Cycle { listeners } => format!(
                "component tooltip appender ordering cycle: {}",
                listeners
                    .iter()
                    .map(Identifier::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            error => error.to_string(),
        })?
        .into_iter()
        .filter_map(|node| match node {
            None => {
                slot += 1;
                None
            }
            Some(index) => Some((slot, index)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use TooltipPlacement::*;

    fn order(input: &[(&str, TooltipPlacement)]) -> Result<Vec<(usize, usize)>, String> {
        let ids = input
            .iter()
            .map(|(id, _)| Identifier::parse_namespaced(id).unwrap())
            .collect::<Vec<_>>();
        sort_tooltip_appenders(ids.iter().zip(input).map(|(id, (_, p))| (id, p, "test")))
    }

    #[test]
    fn empty_component_boundaries_keep_builder_order() {
        let placements = [
            ("test:after", After("minecraft:enchantments".into())),
            ("test:before_bees", Before("minecraft:bees".into())),
            ("test:first", BeforeAll),
            ("test:also_before_bees", Before("minecraft:bees".into())),
            ("test:last", AfterAll),
        ];
        use VanillaTooltipDetail as Detail;
        // BeforeAll blocks the first stock node, so the earlier ready mod can still win.
        // After is also a precedence constraint: ready stock nodes win once they unblock.
        assert_eq!(
            order(&placements).unwrap(),
            [
                (Detail::TropicalFishPattern as usize, 1),
                (Detail::TropicalFishPattern as usize, 2),
                (Detail::Bees as usize, 3),
                (Detail::Damage as usize + 1, 0),
                (Detail::Damage as usize + 1, 4),
            ]
        );
    }

    #[test]
    fn tooltip_graph_resolves_forward_modded_and_new_vanilla_anchors() {
        let placements = [
            ("test:dependent", Before("other:anchor".into())),
            ("other:anchor", Before("minecraft:sign_text_front".into())),
            ("test:second", Before("other:anchor".into())),
            ("test:damage", Before("minecraft:damage".into())),
        ];
        let sign = VanillaTooltipDetail::SignTextFront as usize;
        assert_eq!(
            order(&placements).unwrap(),
            [
                (sign, 0),
                (sign, 2),
                (sign, 1),
                (VanillaTooltipDetail::Damage as usize, 3)
            ]
        );
    }

    #[test]
    fn tooltip_graph_refuses_missing_appender_anchors() {
        let error = order(&[("test:charge", After("minecraft:repair_cost".into()))]).unwrap_err();
        assert_eq!(
            error,
            "mod test: tooltip appender for test:charge has no appender to anchor to at minecraft:repair_cost"
        );
    }

    #[test]
    fn tooltip_graph_refuses_duplicate_vanilla_and_modded_appenders() {
        for placements in [
            [("test:glow", BeforeAll), ("minecraft:damage", AfterAll)],
            [("test:glow", BeforeAll), ("test:glow", AfterAll)],
        ] {
            let error = order(&placements).unwrap_err();
            assert_eq!(
                error,
                format!(
                    "mod test: duplicate tooltip appender for {}",
                    placements[1].0
                )
            );
        }
    }

    #[test]
    fn tooltip_graph_refuses_cycles_and_self_edges() {
        let error = order(&[
            ("test:a", Before("test:b".into())),
            ("test:b", Before("test:a".into())),
            ("test:downstream", After("test:a".into())),
        ])
        .unwrap_err();
        assert!(error.contains("cycle") && error.contains("test:a") && error.contains("test:b"));
        assert!(!error.contains("test:downstream"));
        assert_eq!(
            order(&[("test:self", Before("test:self".into()))]).unwrap_err(),
            "mod test: tooltip appender for test:self cannot anchor to itself",
        );
    }
}
