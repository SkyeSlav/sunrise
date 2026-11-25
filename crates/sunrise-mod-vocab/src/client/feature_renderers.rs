//! Custom feature renderers and per-view typed feature submission
//! (NeoForge 26.3's `RegisterFeatureRenderersEvent` and `RenderPhaseKeys`, vanilla 26.3's
//! `FeatureRenderer`, `FeatureRendererType`, and `FeatureRenderDispatcher`; #1162).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use crate::generated::HookId;
use crate::runtime::discard_registration;
use crate::{ClientHost, Identifier, WorldDrawContext, WorldRenderView};

/// Identifies a vanilla feature render phase (NeoForge 26.3's `RenderPhaseKeys`).
///
/// Submitted nodes are grouped by `(RenderPhaseKey, FeatureRendererType)` and flushed in the
/// phase order of vanilla's `FeatureRenderDispatcher` ([`Self::EXECUTION_ORDER`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: in-process render phase key for client feature submission.
pub enum RenderPhaseKey {
    /// Solid opaque features (`RenderPhaseKeys.SOLID`, `collection.solid`).
    Solid,
    /// Entity ground shadows (`RenderPhaseKeys.SHADOWS`, `collection.shadows`).
    Shadows,
    /// Floating entity name tags (`RenderPhaseKeys.NAME_TAGS`, `collection.nameTags`).
    NameTags,
    /// World-space text submissions (`RenderPhaseKeys.TEXTS`, `collection.texts`).
    Texts,
    /// Block and shape outlines (`RenderPhaseKeys.SHAPE_OUTLINES`, `collection.shapeOutlines`).
    ShapeOutlines,
    /// Translucent blocks and items (`RenderPhaseKeys.TRANSLUCENT_BLOCKS_AND_ITEMS`,
    /// `collection.translucentBlocksAndItems`).
    TranslucentBlocksAndItems,
    /// Translucent entity and block-entity models (`RenderPhaseKeys.TRANSLUCENT_MODELS`,
    /// `collection.translucentModels`).
    TranslucentModels,
    /// Translucent custom geometry (`RenderPhaseKeys.TRANSLUCENT_CUSTOM_GEOMETRY`,
    /// `collection.translucentCustomGeometry`).
    TranslucentCustomGeometry,
    /// Translucent world gizmos (`RenderPhaseKeys.TRANSLUCENT_GIZMOS`,
    /// `collection.translucentGizmos`).
    TranslucentGizmos,
    /// Block-breaking crack overlay (`RenderPhaseKeys.BREAKING_OVERLAY`,
    /// `collection.breakingOverlay`).
    BreakingOverlay,
    /// Water depth mask (`RenderPhaseKeys.WATER_MASK`, `collection.waterMask`).
    WaterMask,
    /// Translucent features drawn after terrain (`RenderPhaseKeys.AFTER_TERRAIN`,
    /// `collection.afterTerrain`).
    AfterTerrain,
    /// Always-on-top gizmos (`RenderPhaseKeys.ALWAYS_ON_TOP`, `collection.alwaysOnTopGizmos`).
    AlwaysOnTop,
    /// Glowing entity and block outline buffer (`RenderPhaseKeys.OUTLINE`, `collection.outline`).
    Outline,
}

impl RenderPhaseKey {
    /// Vanilla 26.3 `FeatureRenderDispatcher` phase execution order
    /// (`executeSolid`, `executeTranslucent`, `executeTranslucentAfterTerrain`,
    /// `executeAlwaysOnTop`, `executeOutline`).
    pub const EXECUTION_ORDER: &'static [Self] = &[
        Self::Solid,
        Self::Shadows,
        Self::TranslucentModels,
        Self::NameTags,
        Self::Texts,
        Self::TranslucentCustomGeometry,
        Self::ShapeOutlines,
        Self::TranslucentGizmos,
        Self::TranslucentBlocksAndItems,
        Self::BreakingOverlay,
        Self::WaterMask,
        Self::AfterTerrain,
        Self::AlwaysOnTop,
        Self::Outline,
    ];
}

/// Typed key for a custom feature renderer and its submitted nodes (vanilla 26.3's
/// `FeatureRendererType<S>`, NeoForge 26.3's `RegisterFeatureRenderersEvent`).
///
/// Declare types as `const` with a namespaced identifier (`"namespace:path"`). Register the
/// type identity during [`Hooks::register`](crate::Hooks::register) via
/// [`Registrar::register_feature_renderer_type`](crate::Registrar::register_feature_renderer_type)
/// so startup refuses any `(id, Rust type)` collision naming both mods and both types, and bind a
/// [`FeatureRenderer<S>`] during
/// [`Hooks::register_feature_renderers`](crate::Hooks::register_feature_renderers).
// no wire: in-process typed key for client feature submission; never crosses the wire.
// exhaustive: fields are private; constructed through FeatureRendererType::new.
pub struct FeatureRendererType<S: Send + Sync + 'static> {
    id: &'static str,
    marker: PhantomData<fn() -> S>,
}

impl<S: Send + Sync + 'static> FeatureRendererType<S> {
    /// Define a typed feature renderer key for the namespaced identifier `id`
    /// (`"namespace:path"`).
    ///
    /// Register this type in [`Hooks::register`](crate::Hooks::register) via
    /// [`Registrar::register_feature_renderer_type`](crate::Registrar::register_feature_renderer_type).
    pub const fn new(id: &'static str) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// The namespaced identifier of this feature renderer type.
    pub const fn id(&self) -> &'static str {
        self.id
    }
}

impl<S: Send + Sync + 'static> Clone for FeatureRendererType<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Send + Sync + 'static> Copy for FeatureRendererType<S> {}

impl<S: Send + Sync + 'static> PartialEq for FeatureRendererType<S> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<S: Send + Sync + 'static> Eq for FeatureRendererType<S> {}

impl<S: Send + Sync + 'static> Hash for FeatureRendererType<S> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<S: Send + Sync + 'static> fmt::Debug for FeatureRendererType<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FeatureRendererType")
            .field("id", &self.id)
            .field("type", &std::any::type_name::<S>())
            .finish()
    }
}

/// Renders a batched slice of custom feature nodes submitted for one [`RenderPhaseKey`]
/// (vanilla 26.3's `FeatureRenderer<Submit>`, NeoForge 26.3's `RegisterFeatureRenderersEvent`).
///
/// Registered once at client startup via
/// [`Hooks::register_feature_renderers`](crate::Hooks::register_feature_renderers). During each
/// world view, the engine flushes submitted nodes in [`RenderPhaseKey::EXECUTION_ORDER`] and
/// invokes [`Self::render`] with all nodes of type `S` submitted to that phase in submission
/// order.
// no wire: process-local client renderer trait.
pub trait FeatureRenderer<S: Send + Sync + 'static>: Send + Sync + 'static {
    /// Render all `nodes` submitted to `phase` for this view.
    fn render(
        &self,
        host: &dyn ClientHost,
        view: &WorldRenderView<'_>,
        phase: RenderPhaseKey,
        nodes: &[S],
        draw: &mut dyn WorldDrawContext,
    );
}

/// Why a [`FeatureRenderer`] registration was refused at client startup.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: client startup registration diagnostic.
pub enum FeatureRegistrationError {
    /// The [`FeatureRendererType`] identifier is not a valid namespaced `"namespace:path"`.
    InvalidId {
        /// The invalid identifier string.
        id: &'static str,
        /// The mod that attempted the registration.
        mod_id: String,
    },
    /// Two registrations targeted the same [`FeatureRendererType`] identifier
    /// (NeoForge's `IllegalStateException("Duplicate renderer registration for ...")`).
    Duplicate {
        /// The colliding feature renderer type identifier.
        id: &'static str,
        /// The mod that registered the type first.
        first_mod: String,
        /// The mod that attempted the duplicate registration.
        second_mod: String,
    },
}

impl fmt::Display for FeatureRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { id, mod_id } => write!(
                f,
                "mod {mod_id} registered feature renderer type {id:?}, which is not a namespaced name"
            ),
            Self::Duplicate {
                id,
                first_mod,
                second_mod,
            } => write!(
                f,
                "duplicate feature renderer registration for {id}: registered by {first_mod} and {second_mod}"
            ),
        }
    }
}

impl std::error::Error for FeatureRegistrationError {}

/// Why submitting a custom feature node via `WorldDrawContext::submit_feature` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: in-process feature submission refusal.
pub enum FeatureSubmitError {
    /// No [`FeatureRenderer`] is registered for this [`FeatureRendererType`].
    Unregistered,
    /// The submitted Rust type does not match the [`FeatureRendererType`]'s registered type.
    TypeMismatch,
    /// This draw context does not accept custom feature submissions.
    Unsupported,
}

impl fmt::Display for FeatureSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unregistered => f.write_str("no feature renderer registered for this type"),
            Self::TypeMismatch => {
                f.write_str("submitted feature node type does not match registration")
            }
            Self::Unsupported => {
                f.write_str("this draw context does not accept feature submissions")
            }
        }
    }
}

impl std::error::Error for FeatureSubmitError {}

type ErasedFeatureRender = dyn Fn(
        &dyn ClientHost,
        &WorldRenderView<'_>,
        RenderPhaseKey,
        &(dyn Any + Send + Sync),
        &mut dyn WorldDrawContext,
    ) + Send
    + Sync
    + 'static;

/// Registrar passed to
/// [`Hooks::register_feature_renderers`](crate::Hooks::register_feature_renderers).
///
/// Call `event.renderers.register(&TYPE, renderer)` with a [`FeatureRendererType<S>`] and a
/// [`FeatureRenderer<S>`]. Duplicate registrations for the same type identifier are refused at
/// startup naming both mods, matching NeoForge 26.3's `RegisterFeatureRenderersEvent`.
// no wire: in-process client startup registration surface.
pub trait FeatureRenderers {
    /// Register a type-erased feature renderer entry.
    // hidden: object-safe backing method for `dyn FeatureRenderers::register`.
    #[doc(hidden)]
    fn register_raw(
        &mut self,
        id: &'static str,
        node_type_id: TypeId,
        node_type_name: &'static str,
        render: Box<ErasedFeatureRender>,
    );
}

impl dyn FeatureRenderers + '_ {
    /// Register `renderer` for nodes of type `ty` (NeoForge's
    /// `RegisterFeatureRenderersEvent.register(FeatureRendererType, FeatureRenderer)`).
    ///
    /// Also register `ty` in [`Hooks::register`](crate::Hooks::register) via
    /// [`Registrar::register_feature_renderer_type`](crate::Registrar::register_feature_renderer_type)
    /// so cross-mod `(id, Rust type)` collisions are caught by the startup identity table.
    /// Registering a second renderer for the same `ty.id()` fails client startup naming both mods.
    pub fn register<S, R>(&mut self, ty: &FeatureRendererType<S>, renderer: R)
    where
        S: Send + Sync + 'static,
        R: FeatureRenderer<S>,
    {
        let render: Box<ErasedFeatureRender> = Box::new(move |host, view, phase, nodes, draw| {
            if let Some(typed) = nodes.downcast_ref::<Vec<S>>() {
                renderer.render(host, view, phase, typed.as_slice(), draw);
            }
        });
        self.register_raw(
            ty.id(),
            TypeId::of::<S>(),
            std::any::type_name::<S>(),
            render,
        );
    }
}

const REGISTRATION: (HookId, &str) = (
    HookId::RegisterFeatureRenderers,
    "register_feature_renderers",
);

struct RawFeatureRegistration {
    id: &'static str,
    node_type_id: TypeId,
    node_type_name: &'static str,
    render: Box<ErasedFeatureRender>,
}

#[derive(Default)]
struct FeatureRendererEdits(Vec<RawFeatureRegistration>);

impl FeatureRenderers for FeatureRendererEdits {
    fn register_raw(
        &mut self,
        id: &'static str,
        node_type_id: TypeId,
        node_type_name: &'static str,
        render: Box<ErasedFeatureRender>,
    ) {
        self.0.push(RawFeatureRegistration {
            id,
            node_type_id,
            node_type_name,
            render,
        });
    }
}

/// One registered feature renderer and its metadata.
// hidden: engine-owned registration entry; mods implement `FeatureRenderer<S>`.
// no wire: process-local client renderer entry.
// exhaustive: fields are private; constructed by `FeatureRendererRegistry`.
#[doc(hidden)]
pub struct RegisteredFeatureRenderer {
    owner: Option<&'static str>,
    node_type_id: TypeId,
    node_type_name: &'static str,
    render: Box<ErasedFeatureRender>,
}

impl RegisteredFeatureRenderer {
    /// The mod ID that registered this feature renderer, or `None` outside a mod call.
    pub fn owner(&self) -> Option<&'static str> {
        self.owner
    }

    /// The `TypeId` of the submit node type `S` this renderer expects.
    pub fn node_type_id(&self) -> TypeId {
        self.node_type_id
    }

    /// The Rust type name of the submit node type `S` this renderer expects.
    pub fn node_type_name(&self) -> &'static str {
        self.node_type_name
    }

    /// Invoke this renderer with a type-erased `Vec<S>` node batch.
    pub fn render(
        &self,
        host: &dyn ClientHost,
        view: &WorldRenderView<'_>,
        phase: RenderPhaseKey,
        nodes: &(dyn Any + Send + Sync),
        draw: &mut dyn WorldDrawContext,
    ) {
        (self.render)(host, view, phase, nodes, draw);
    }
}

/// Resolved table of registered feature renderers built once at client startup.
// hidden: engine-owned startup table; mods register through `Hooks::register_feature_renderers`.
// no wire: process-local client renderer table.
// exhaustive: fields are private; built by `FeatureRendererRegistry::into_table`.
#[doc(hidden)]
#[derive(Default)]
pub struct FeatureRendererTable {
    renderers: HashMap<&'static str, RegisteredFeatureRenderer>,
}

impl FeatureRendererTable {
    /// Look up the registered feature renderer for `id`.
    pub fn get(&self, id: &str) -> Option<&RegisteredFeatureRenderer> {
        self.renderers.get(id)
    }

    /// Iterate over all registered feature renderers.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &RegisteredFeatureRenderer)> {
        self.renderers.iter().map(|(&id, renderer)| (id, renderer))
    }

    /// Number of registered feature renderers.
    pub fn len(&self) -> usize {
        self.renderers.len()
    }

    /// Whether no feature renderers are registered.
    pub fn is_empty(&self) -> bool {
        self.renderers.is_empty()
    }
}

/// Staged startup collector for [`Hooks::register_feature_renderers`].
// hidden: the engine stages one listener at a time; mods receive only `dyn FeatureRenderers`.
// no wire: process-local startup collector.
// exhaustive: fields are private.
#[doc(hidden)]
#[derive(Default)]
pub struct FeatureRendererRegistry {
    renderers: HashMap<&'static str, RegisteredFeatureRenderer>,
    errors: Vec<FeatureRegistrationError>,
}

impl FeatureRenderers for FeatureRendererRegistry {
    fn register_raw(
        &mut self,
        id: &'static str,
        node_type_id: TypeId,
        node_type_name: &'static str,
        render: Box<ErasedFeatureRender>,
    ) {
        self.publish([RawFeatureRegistration {
            id,
            node_type_id,
            node_type_name,
            render,
        }]);
    }
}

impl FeatureRendererRegistry {
    /// Stage one listener's registrations; a panic discards that listener's entire batch.
    // hidden: called by generated hook glue in the game crate.
    #[doc(hidden)]
    pub fn register_listener(&mut self, listen: &mut dyn FnMut(&mut dyn FeatureRenderers) -> bool) {
        let mut edits = FeatureRendererEdits::default();
        if listen(&mut edits) {
            self.publish(edits.0);
        } else {
            let owner = crate::content::crash::current_native_mod();
            for entry in edits.0 {
                discard_registration(REGISTRATION, owner, entry.render);
            }
        }
    }

    fn publish(&mut self, edits: impl IntoIterator<Item = RawFeatureRegistration>) {
        let owner = crate::content::crash::current_native_mod();
        let owner_label = owner.unwrap_or("the game");
        for entry in edits {
            if Identifier::parse_namespaced(entry.id).is_none() {
                self.errors.push(FeatureRegistrationError::InvalidId {
                    id: entry.id,
                    mod_id: owner_label.to_string(),
                });
                discard_registration(REGISTRATION, owner, entry.render);
                continue;
            }
            if let Some(existing) = self.renderers.get(entry.id) {
                self.errors.push(FeatureRegistrationError::Duplicate {
                    id: entry.id,
                    first_mod: existing.owner.unwrap_or("the game").to_string(),
                    second_mod: owner_label.to_string(),
                });
                discard_registration(REGISTRATION, owner, entry.render);
                continue;
            }
            self.renderers.insert(
                entry.id,
                RegisteredFeatureRenderer {
                    owner,
                    node_type_id: entry.node_type_id,
                    node_type_name: entry.node_type_name,
                    render: entry.render,
                },
            );
        }
    }

    /// Consume the registry into a resolved [`FeatureRendererTable`], or fail with every
    /// startup registration error collected.
    pub fn into_table(self) -> Result<FeatureRendererTable, Vec<FeatureRegistrationError>> {
        if self.errors.is_empty() {
            Ok(FeatureRendererTable {
                renderers: self.renderers,
            })
        } else {
            Err(self.errors)
        }
    }
}

/// Per-view typed arena of submitted custom feature nodes grouped by `(RenderPhaseKey, id)`.
///
/// Cleared before each view and dropped when the view ends so mod-owned values never leak across
/// frames and are dropped before any mod library unloads.
// hidden: engine-owned per-view submission queue; mods submit via `WorldDrawContext::submit_feature`.
// no wire: process-local per-view submission queue.
// exhaustive: fields are private.
#[doc(hidden)]
#[derive(Default)]
pub struct FeatureSubmitQueue {
    phases: HashMap<RenderPhaseKey, Vec<(&'static str, Box<dyn Any + Send + Sync>)>>,
}

impl FeatureSubmitQueue {
    /// Empty queue; performs no heap allocation until a feature node is submitted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every queued node across all phases immediately.
    pub fn clear(&mut self) {
        self.phases.clear();
    }

    /// Whether no feature nodes are queued in any phase.
    pub fn is_empty(&self) -> bool {
        self.phases.values().all(Vec::is_empty)
    }

    /// Validate `(feature_type, type_id)` against `table` and append into `phase`'s typed bucket.
    pub fn submit_raw(
        &mut self,
        table: &FeatureRendererTable,
        feature_type: &'static str,
        type_id: TypeId,
        phase: RenderPhaseKey,
        push: &mut dyn FnMut(
            Option<&mut (dyn Any + Send + Sync)>,
        )
            -> Result<Option<Box<dyn Any + Send + Sync>>, FeatureSubmitError>,
    ) -> Result<(), FeatureSubmitError> {
        let Some(registered) = table.get(feature_type) else {
            return Err(FeatureSubmitError::Unregistered);
        };
        if registered.node_type_id != type_id {
            return Err(FeatureSubmitError::TypeMismatch);
        }
        let buckets = self.phases.entry(phase).or_default();
        if let Some((_, existing)) = buckets.iter_mut().find(|(id, _)| *id == feature_type) {
            let _ = push(Some(&mut **existing))?;
        } else if let Some(created) = push(None)? {
            buckets.push((feature_type, created));
        }
        Ok(())
    }

    /// Drain all queued `(feature_type, Vec<S>)` groups for `phase` in submission order.
    pub fn take_phase(
        &mut self,
        phase: RenderPhaseKey,
    ) -> Vec<(&'static str, Box<dyn Any + Send + Sync>)> {
        self.phases.remove(&phase).unwrap_or_default()
    }

    /// Flush all queued phases in [`RenderPhaseKey::EXECUTION_ORDER`] into `table`.
    pub fn flush(
        &mut self,
        table: &FeatureRendererTable,
        host: &dyn ClientHost,
        view: &WorldRenderView<'_>,
        draw: &mut dyn WorldDrawContext,
    ) {
        for &phase in RenderPhaseKey::EXECUTION_ORDER {
            let groups = self.take_phase(phase);
            for (id, nodes) in groups {
                if let Some(renderer) = table.get(id) {
                    renderer.render(host, view, phase, &*nodes, draw);
                }
            }
        }
        self.clear();
    }
}

impl dyn WorldDrawContext + '_ {
    /// Submit a typed custom feature `node` for `ty` into `phase` (NeoForge 26.3's
    /// `OrderedSubmitNodeCollectorExtension.submitSpecial(RenderPhaseKey, SubmitNode)`).
    ///
    /// Nodes submitted to the same `(phase, ty)` during one world view are batched in submission
    /// order and passed as `&[S]` to the registered [`FeatureRenderer<S>`] when phases flush in
    /// [`RenderPhaseKey::EXECUTION_ORDER`]. Returns [`FeatureSubmitError::Unregistered`] if no
    /// renderer is registered for `ty`, or [`FeatureSubmitError::TypeMismatch`] if `S` does not
    /// match the registered type.
    pub fn submit_feature<S: Send + Sync + 'static>(
        &mut self,
        ty: &FeatureRendererType<S>,
        phase: RenderPhaseKey,
        node: S,
    ) -> Result<(), FeatureSubmitError> {
        let mut node = Some(node);
        self.submit_feature_raw(ty.id(), TypeId::of::<S>(), phase, &mut |existing| {
            let value = node.take().ok_or(FeatureSubmitError::Unsupported)?;
            match existing {
                Some(any) => {
                    let vec = any
                        .downcast_mut::<Vec<S>>()
                        .ok_or(FeatureSubmitError::TypeMismatch)?;
                    vec.push(value);
                    Ok(None)
                }
                None => Ok(Some(Box::new(vec![value]) as Box<dyn Any + Send + Sync>)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, WorldRenderView};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct DummyHost;
    impl crate::Host for DummyHost {}
    mod_api::impl_host_areas!(DummyHost);
    impl crate::ClientHost for DummyHost {}
    mod_api::impl_client_host_areas!(DummyHost);

    struct QueueDraw<'a> {
        table: &'a FeatureRendererTable,
        queue: &'a mut FeatureSubmitQueue,
    }
    impl WorldDrawContext for QueueDraw<'_> {
        fn submit_feature_raw(
            &mut self,
            feature_type: &'static str,
            type_id: TypeId,
            phase: RenderPhaseKey,
            push: &mut dyn FnMut(
                Option<&mut (dyn Any + Send + Sync)>,
            )
                -> Result<Option<Box<dyn Any + Send + Sync>>, FeatureSubmitError>,
        ) -> Result<(), FeatureSubmitError> {
            self.queue
                .submit_raw(self.table, feature_type, type_id, phase, push)
        }
    }

    struct SinkDraw;
    impl WorldDrawContext for SinkDraw {}

    fn test_camera() -> Camera {
        Camera {
            position: [0.0, 64.0, 0.0],
            eye_height: 1.62,
            partial_tick: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            third_person: false,
            mirrored: false,
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct BeamNode(u32);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct WrongNode(String);

    const BEAM_TYPE: FeatureRendererType<BeamNode> = FeatureRendererType::new("example:beam");
    const WRONG_BEAM_TYPE: FeatureRendererType<WrongNode> =
        FeatureRendererType::new("example:beam");
    const UNREGISTERED_TYPE: FeatureRendererType<BeamNode> =
        FeatureRendererType::new("example:missing");
    const BAD_ID_TYPE: FeatureRendererType<BeamNode> =
        FeatureRendererType::new("missing_namespace");

    struct RecordingRenderer {
        log: Arc<Mutex<Vec<(RenderPhaseKey, Vec<u32>)>>>,
    }

    impl FeatureRenderer<BeamNode> for RecordingRenderer {
        fn render(
            &self,
            _: &dyn ClientHost,
            _: &WorldRenderView<'_>,
            phase: RenderPhaseKey,
            nodes: &[BeamNode],
            _: &mut dyn WorldDrawContext,
        ) {
            self.log
                .lock()
                .unwrap()
                .push((phase, nodes.iter().map(|n| n.0).collect()));
        }
    }

    #[test]
    fn execution_order_covers_all_fourteen_neoforge_phases() {
        assert_eq!(RenderPhaseKey::EXECUTION_ORDER.len(), 14);
        assert_eq!(RenderPhaseKey::EXECUTION_ORDER[0], RenderPhaseKey::Solid);
        let solid_pos = RenderPhaseKey::EXECUTION_ORDER
            .iter()
            .position(|&p| p == RenderPhaseKey::Solid)
            .unwrap();
        let translucent_pos = RenderPhaseKey::EXECUTION_ORDER
            .iter()
            .position(|&p| p == RenderPhaseKey::TranslucentCustomGeometry)
            .unwrap();
        let after_terrain_pos = RenderPhaseKey::EXECUTION_ORDER
            .iter()
            .position(|&p| p == RenderPhaseKey::AfterTerrain)
            .unwrap();
        let always_on_top_pos = RenderPhaseKey::EXECUTION_ORDER
            .iter()
            .position(|&p| p == RenderPhaseKey::AlwaysOnTop)
            .unwrap();
        let outline_pos = RenderPhaseKey::EXECUTION_ORDER
            .iter()
            .position(|&p| p == RenderPhaseKey::Outline)
            .unwrap();
        assert!(solid_pos < translucent_pos);
        assert!(translucent_pos < after_terrain_pos);
        assert!(after_terrain_pos < always_on_top_pos);
        assert!(always_on_top_pos < outline_pos);
    }

    #[test]
    fn batched_nodes_flush_in_phase_order_preserving_submission_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut registry = FeatureRendererRegistry::default();
        let reg: &mut dyn FeatureRenderers = &mut registry;
        reg.register(
            &BEAM_TYPE,
            RecordingRenderer {
                log: Arc::clone(&log),
            },
        );
        let table = registry.into_table().unwrap();

        let mut queue = FeatureSubmitQueue::new();
        {
            let mut draw = QueueDraw {
                table: &table,
                queue: &mut queue,
            };
            let draw_dyn: &mut dyn WorldDrawContext = &mut draw;
            // Submit translucent before solid to verify phase-order flushing.
            draw_dyn
                .submit_feature(
                    &BEAM_TYPE,
                    RenderPhaseKey::TranslucentCustomGeometry,
                    BeamNode(10),
                )
                .unwrap();
            draw_dyn
                .submit_feature(&BEAM_TYPE, RenderPhaseKey::Solid, BeamNode(1))
                .unwrap();
            draw_dyn
                .submit_feature(
                    &BEAM_TYPE,
                    RenderPhaseKey::TranslucentCustomGeometry,
                    BeamNode(20),
                )
                .unwrap();
            draw_dyn
                .submit_feature(&BEAM_TYPE, RenderPhaseKey::Solid, BeamNode(2))
                .unwrap();
            draw_dyn
                .submit_feature(&BEAM_TYPE, RenderPhaseKey::Outline, BeamNode(99))
                .unwrap();
        }

        let camera = test_camera();
        let view = WorldRenderView::new(&camera, 0.5, 100, false, &[]);
        let mut sink = SinkDraw;
        queue.flush(&table, &DummyHost, &view, &mut sink);

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                (RenderPhaseKey::Solid, vec![1, 2]),
                (RenderPhaseKey::TranslucentCustomGeometry, vec![10, 20]),
                (RenderPhaseKey::Outline, vec![99]),
            ]
        );
        assert!(queue.is_empty(), "queue must be empty after flush");
    }

    #[test]
    fn duplicate_and_invalid_registrations_are_refused() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut registry = FeatureRendererRegistry::default();
        let reg: &mut dyn FeatureRenderers = &mut registry;
        reg.register(
            &BEAM_TYPE,
            RecordingRenderer {
                log: Arc::clone(&log),
            },
        );
        reg.register(
            &BEAM_TYPE,
            RecordingRenderer {
                log: Arc::clone(&log),
            },
        );
        reg.register(&BAD_ID_TYPE, RecordingRenderer { log });

        let errors = registry.into_table().unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(matches!(
            errors[0],
            FeatureRegistrationError::Duplicate {
                id: "example:beam",
                ..
            }
        ));
        assert!(matches!(
            errors[1],
            FeatureRegistrationError::InvalidId {
                id: "missing_namespace",
                ..
            }
        ));
    }

    #[test]
    fn unregistered_and_type_mismatch_submissions_return_errors_without_panicking() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut registry = FeatureRendererRegistry::default();
        let reg: &mut dyn FeatureRenderers = &mut registry;
        reg.register(&BEAM_TYPE, RecordingRenderer { log });
        let table = registry.into_table().unwrap();

        let mut queue = FeatureSubmitQueue::new();
        let mut draw = QueueDraw {
            table: &table,
            queue: &mut queue,
        };
        let draw_dyn: &mut dyn WorldDrawContext = &mut draw;

        assert_eq!(
            draw_dyn.submit_feature(&UNREGISTERED_TYPE, RenderPhaseKey::Solid, BeamNode(1)),
            Err(FeatureSubmitError::Unregistered)
        );
        assert_eq!(
            draw_dyn.submit_feature(
                &WRONG_BEAM_TYPE,
                RenderPhaseKey::Solid,
                WrongNode("bad".into()),
            ),
            Err(FeatureSubmitError::TypeMismatch)
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn submitted_values_drop_on_clear_and_queue_drop() {
        struct DropNode(Arc<AtomicBool>);
        impl Drop for DropNode {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        struct NoopRenderer;
        impl FeatureRenderer<DropNode> for NoopRenderer {
            fn render(
                &self,
                _: &dyn ClientHost,
                _: &WorldRenderView<'_>,
                _: RenderPhaseKey,
                _: &[DropNode],
                _: &mut dyn WorldDrawContext,
            ) {
            }
        }
        const DROP_TYPE: FeatureRendererType<DropNode> = FeatureRendererType::new("example:drop");

        let mut registry = FeatureRendererRegistry::default();
        let reg: &mut dyn FeatureRenderers = &mut registry;
        reg.register(&DROP_TYPE, NoopRenderer);
        let table = registry.into_table().unwrap();

        let flag_clear = Arc::new(AtomicBool::new(false));
        let flag_drop = Arc::new(AtomicBool::new(false));
        {
            let mut queue = FeatureSubmitQueue::new();
            {
                let mut draw = QueueDraw {
                    table: &table,
                    queue: &mut queue,
                };
                let draw_dyn: &mut dyn WorldDrawContext = &mut draw;
                draw_dyn
                    .submit_feature(
                        &DROP_TYPE,
                        RenderPhaseKey::Solid,
                        DropNode(Arc::clone(&flag_clear)),
                    )
                    .unwrap();
            }
            assert!(!flag_clear.load(Ordering::SeqCst));
            queue.clear();
            assert!(flag_clear.load(Ordering::SeqCst));

            {
                let mut draw = QueueDraw {
                    table: &table,
                    queue: &mut queue,
                };
                let draw_dyn: &mut dyn WorldDrawContext = &mut draw;
                draw_dyn
                    .submit_feature(
                        &DROP_TYPE,
                        RenderPhaseKey::AfterTerrain,
                        DropNode(Arc::clone(&flag_drop)),
                    )
                    .unwrap();
            }
            assert!(!flag_drop.load(Ordering::SeqCst));
        }
        assert!(flag_drop.load(Ordering::SeqCst));
    }
}
