//! Load-phase data-file condition registration.
pub trait Hooks {
    /// Register named conditions once at mod load, before server data admission.
    /// Each handler is a raw-JSON handler: it reads the condition's JSON value.
    /// IDs may use any namespace, but built-in and duplicate IDs refuse mod load.
    /// Native and wasm handlers share the same route. Foreign registrations
    /// do not grant namespace ownership for unknown-ID handling.
    #[hook(
        side = "server",
        version = 2,
        contract = "collect",
        out = "conditions",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_load_conditions();
}
