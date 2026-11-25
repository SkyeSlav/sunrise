//! Client custom environment effect renderer registration.

pub trait Hooks {
    /// Register custom skybox, clouds, and snow/rain renderers once at client startup
    /// (NeoForge 26.3's `RegisterCustomEnvironmentEffectRendererEvent`). Call
    /// `event.register_skybox(id, renderer)`, `event.register_clouds(id, renderer)`, or
    /// `event.register_weather(id, renderer)` to bind a renderer to an [`Identifier`] referenced
    /// by the `neoforge:custom_skybox`, `neoforge:custom_clouds`, or
    /// `neoforge:custom_weather_effects` environment attribute.
    ///
    /// Registering the reserved default identifier (`minecraft:default`) is refused immediately
    /// with [`EnvironmentEffectRegistrationError::DefaultId`]. Duplicate identifiers within the
    /// same effect slot fail client startup, naming both mods. Active renderers receive only
    /// extracted view state ([`WorldRenderView`], [`SkyRenderState`], [`CloudsRenderState`],
    /// [`WeatherRenderState`]) and a [`WorldDrawContext`]; extract any world data needed during
    /// drawing in `Hooks::extract_level_render_state`. Returning `true` from a renderer
    /// suppresses vanilla's corresponding skybox, clouds, or weather draw for that view.
    #[notes = r#"## Custom environment effect renderers

`Hooks::register_custom_environment_effect_renderers` is the client startup registration
counterpart of NeoForge 26.3's `RegisterCustomEnvironmentEffectRendererEvent`. Register custom
renderers with `event.register_skybox(id, renderer)`, `event.register_clouds(id, renderer)`, and
`event.register_weather(id, renderer)`. A dimension type or biome selects an active renderer
through `neoforge:custom_skybox`, `neoforge:custom_clouds`, or `neoforge:custom_weather_effects`
(`NeoForgeEnvironmentAttributes`), which default to `minecraft:default`.

Registering `minecraft:default` is refused immediately, and duplicate registrations for the same
`(slot, id)` fail client startup naming both mods. Registrations persist across resource reloads
and world changes. Vanilla dimensions keep the default attribute values and run zero callbacks.

Renderers read only extracted view state (`WorldRenderView` including `RenderDataKey` values from
`Hooks::extract_level_render_state`, plus `SkyRenderState`, `CloudsRenderState`, or
`WeatherRenderState`) and draw through `WorldDrawContext`; no live `ClientHost` world queries are
available during drawing. Returning `true` suppresses vanilla's skybox, clouds, or weather pass
for that view, while `false` lets vanilla draw as well. A panicking renderer is disabled for the
session and falls back to vanilla rendering."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "renderers",
        registration_append = "attributed",
        context = "registration",
        neoforge = "RegisterCustomEnvironmentEffectRendererEvent"
    )]
    fn register_custom_environment_effect_renderers(
        renderers: &mut Vec<mod_api::EnvironmentEffectRendererRegistration>,
    );
}
