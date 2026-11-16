//! `LevelRef::place_feature` from a native and a wasm mod reaches the server's
//! backend with the level, the feature id and the position intact, and each named
//! refusal comes back as the native call returns it.
use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::{LevelRef, PlaceFeatureRefusal, WorldEpoch};

thread_local! {
    static ASKED: std::cell::RefCell<Vec<(LevelRef, String, [i32; 3])>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Every refusal, in the order the asks at x 3 to 7 get them.
const REFUSALS: [PlaceFeatureRefusal; 5] = [
    PlaceFeatureRefusal::Stale,
    PlaceFeatureRefusal::Client,
    PlaceFeatureRefusal::NotInPlace,
    PlaceFeatureRefusal::NotLoaded,
    PlaceFeatureRefusal::UnknownFeature,
];

/// What the server answers: x 1 placed, x 2 reported failure, 3 to 7 are refused.
fn answer(pos: [i32; 3]) -> Result<bool, PlaceFeatureRefusal> {
    match pos[0] {
        1 => Ok(true),
        2 => Ok(false),
        x => Err(REFUSALS[x as usize - 3]),
    }
}

struct Server;
impl mod_api::Host for Server {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
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
mod_api::impl_backend_mut_areas!(Server, except(level_worldgen));
impl mod_api::prelude::LevelWorldgenBackendMut for Server {
    fn level_place_feature(
        &mut self,
        level: LevelRef,
        feature: &str,
        pos: [i32; 3],
    ) -> Result<bool, PlaceFeatureRefusal> {
        ASKED.with_borrow_mut(|asked| asked.push((level, feature.to_owned(), pos)));
        answer(pos)
    }
}

struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

fn asks() -> Vec<(LevelRef, String, [i32; 3])> {
    let level = LevelRef::new(WorldEpoch::new(4), mod_api::DimensionKey::OVERWORLD);
    (1..=7)
        .map(|x| (level, format!("mymod:tree_{x}"), [x, 64, -x]))
        .collect()
}

#[test]
fn a_native_mods_place_feature_dispatches_to_the_server_backend() {
    ASKED.with_borrow_mut(Vec::clear);
    let mut server = Server;
    for (level, feature, pos) in asks() {
        assert_eq!(level.place_feature(&mut server, &feature, pos), answer(pos));
    }
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn a_wasm_mods_place_feature_crosses_whole_and_its_refusal_comes_back() {
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
        for (level, feature, pos) in asks() {
            assert_eq!(
                level.place_feature(&mut guest::Guest, &feature, pos),
                answer(pos),
                "{feature} came back as the server answered"
            );
        }
        guest::set_transport(None);
    }
    assert_eq!(ASKED.with_borrow_mut(std::mem::take), asks());
}

#[test]
fn a_place_feature_with_no_backend_is_refused_as_client() {
    let mut absent = NoBackend;
    for (level, feature, pos) in asks() {
        assert_eq!(
            level.place_feature(&mut absent, &feature, pos),
            Err(PlaceFeatureRefusal::Client)
        );
    }
}

#[test]
fn every_place_feature_refusal_round_trips_on_the_wire() {
    for refusal in REFUSALS {
        let mut w = Writer::new();
        w.put(&Err::<bool, _>(refusal));
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        let back: Result<bool, PlaceFeatureRefusal> = r.take().unwrap();
        assert_eq!(back, Err(refusal));
    }
}
