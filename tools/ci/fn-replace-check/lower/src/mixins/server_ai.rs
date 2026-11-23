fn server_ai(m: &Mob) -> ::core::ops::ControlFlow<::core::ops::ControlFlow<AiPause>> {
    static SEEN: std::sync::Once = std::sync::Once::new();
    let _ = m;
    SEEN.call_once(|| fnr_lower::note("HEAD mobs::ai::server_ai_step"));
    ::core::ops::ControlFlow::Continue(())
}
