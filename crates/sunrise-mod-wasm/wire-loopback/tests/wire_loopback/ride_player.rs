//! `EntityRef::ride_player` from a wasm mod reaches the server's backend
//! with its rider, player and `force` intact, and each named refusal comes
//! back as the native call returns it (#4456).
use super::*;
use mod_api::__private::game::BackendMutRefOf;
use mod_api::{EntityRef, RidePlayerRefusal, WorldEpoch};

thread_local! {
    static ASKED: std::cell::RefCell<Vec<(EntityRef, EntityRef, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// What the server answers: the rider's id picks the refusal, 1 seats it.
fn answer(rider: EntityRef) -> Result<(), RidePlayerRefusal> {
    match rider.id() {
        1 => Ok(()),
        2 => Err(RidePlayerRefusal::Full),
        3 => Err(RidePlayerRefusal::PlayerRider),
        4 => Err(RidePlayerRefusal::Cycle),
        5 => Err(RidePlayerRefusal::NotAPlayer),
        _ => Err(RidePlayerRefusal::Canceled),
    }
}

struct Server;
impl mod_api::Host for Server {}
mod_api::impl_host_areas!(Server);
impl mod_api::ServerHost for Server {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(Server);
impl mod_api::Backend for Server {}
mod_api::impl_backend_areas!(Server);
impl mod_api::BackendMut for Server {}
mod_api::impl_backend_mut_areas!(Server, except(entity_riding));
impl mod_api::prelude::EntityRidingBackendMut for Server {
    fn entity_ride_player(
        &mut self,
        entity: EntityRef,
        player: EntityRef,
        force: bool,
    ) -> Result<(), RidePlayerRefusal> {
        ASKED.with_borrow_mut(|asked| asked.push((entity, player, force)));
        answer(entity)
    }
}

struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

/// The asks: a piggyback pickup, a forced one onto a full player, and the
/// refusals a port must tell apart.
fn asks() -> Vec<(EntityRef, EntityRef, bool)> {
    let epoch = WorldEpoch::new(9);
    let player = EntityRef::new(epoch, 40);
    (1..=6)
        .map(|rider| (EntityRef::new(epoch, rider), player, rider % 2 == 0))
        .collect()
}

#[test]
fn a_wasm_mods_ride_player_crosses_whole_and_its_refusal_comes_back() {
    // The neighbours' guards: per-thread journal, the phase scope and `TransportScope`.
    ASKED.with_borrow_mut(Vec::clear);
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut server = Server;
    {
        let host = &mut server;
        let mut frame = boundary::Frame::default();
        frame.set_server_host_mut(host);
        let mut cx = Box::new(boundary::Cx::default());
        cx.frames.push(frame);
        let pointer = &mut *cx as *mut boundary::Cx;
        // SAFETY: the context and borrowed host outlive synchronous proxy calls.
        let transport = unsafe { host_dispatch::transport(pointer) };
        let previous =
            LOOPBACK_TRANSPORT.with(|active| active.borrow_mut().replace(transport.clone()));
        let _restore = TransportScope(previous);
        guest::set_transport(Some(transport));
        for (rider, player, force) in asks() {
            assert_eq!(
                rider.ride_player(&mut guest::Guest, player, force),
                answer(rider),
                "rider {} came back as the server answered",
                rider.id()
            );
        }
        guest::set_transport(None);
    }
    // The server was handed exactly what the guest asked, in order.
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn ride_player_with_no_backend_is_refused_as_no_entity() {
    let mut absent = NoBackend;
    for (rider, player, force) in asks() {
        assert_eq!(
            rider.ride_player(&mut absent, player, force),
            Err(RidePlayerRefusal::NoEntity)
        );
    }
}

#[test]
fn every_ride_player_refusal_round_trips_on_the_wire() {
    for refusal in [
        RidePlayerRefusal::NoEntity,
        RidePlayerRefusal::NotAPlayer,
        RidePlayerRefusal::PlayerRider,
        RidePlayerRefusal::UnsupportedRider,
        RidePlayerRefusal::SameEntity,
        RidePlayerRefusal::AlreadyRiding,
        RidePlayerRefusal::Cycle,
        RidePlayerRefusal::Full,
        RidePlayerRefusal::CannotRide,
        RidePlayerRefusal::Canceled,
    ] {
        let mut w = Writer::new();
        w.put(&Err::<(), _>(refusal));
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let back: Result<(), RidePlayerRefusal> = r.take().unwrap();
        assert_eq!(back, Err(refusal));
    }
}
