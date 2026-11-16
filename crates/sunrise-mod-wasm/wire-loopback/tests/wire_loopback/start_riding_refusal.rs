//! `EntityRef::start_riding` from a wasm mod reaches the server's
//! backend with its rider, vehicle and `force` intact, and each named refusal
//! comes back as the native call returns it (#4456 part 1).
use super::*;
use mod_api::__private::game::BackendMutRefOf;
use mod_api::{EntityRef, StartRidingRefusal, WorldEpoch};

thread_local! {
    static ASKED: std::cell::RefCell<Vec<(EntityRef, EntityRef, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// What the server answers: the rider's id picks the refusal, 1 seats it.
fn answer(rider: EntityRef) -> Result<(), StartRidingRefusal> {
    match rider.id() {
        1 => Ok(()),
        2 => Err(StartRidingRefusal::PlayerVehicle),
        3 => Err(StartRidingRefusal::Cycle),
        4 => Err(StartRidingRefusal::NoEntity),
        5 => Err(StartRidingRefusal::Full),
        _ => Err(StartRidingRefusal::Canceled),
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
    fn entity_start_riding(
        &mut self,
        entity: EntityRef,
        vehicle: EntityRef,
        force: bool,
    ) -> Result<(), StartRidingRefusal> {
        ASKED.with_borrow_mut(|asked| asked.push((entity, vehicle, force)));
        answer(entity)
    }
}

struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

/// The asks: a mount, a forced one onto a player, and the refusals a port
/// must tell apart.
fn asks() -> Vec<(EntityRef, EntityRef, bool)> {
    let epoch = WorldEpoch::new(9);
    let vehicle = EntityRef::new(epoch, 40);
    (1..=6)
        .map(|rider| (EntityRef::new(epoch, rider), vehicle, rider % 2 == 0))
        .collect()
}

#[test]
fn a_wasm_mods_start_riding_crosses_whole_and_its_refusal_comes_back() {
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
        for (rider, vehicle, force) in asks() {
            assert_eq!(
                rider.start_riding(&mut guest::Guest, vehicle, force),
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
fn start_riding_with_no_backend_is_refused_as_no_entity() {
    let mut absent = NoBackend;
    for (rider, vehicle, force) in asks() {
        assert_eq!(
            rider.start_riding(&mut absent, vehicle, force),
            Err(StartRidingRefusal::NoEntity)
        );
    }
}

#[test]
fn every_start_riding_refusal_round_trips_on_the_wire() {
    for refusal in [
        StartRidingRefusal::NoEntity,
        StartRidingRefusal::PlayerVehicle,
        StartRidingRefusal::UnsupportedRider,
        StartRidingRefusal::UnsupportedVehicle,
        StartRidingRefusal::SameEntity,
        StartRidingRefusal::AlreadyRiding,
        StartRidingRefusal::Cycle,
        StartRidingRefusal::CannotRide,
        StartRidingRefusal::Full,
        StartRidingRefusal::Canceled,
    ] {
        let mut w = Writer::new();
        w.put(&Err::<(), _>(refusal));
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let back: Result<(), StartRidingRefusal> = r.take().unwrap();
        assert_eq!(back, Err(refusal));
    }
}
