//! Named timings (wall and thread CPU) of a frame or tick, on with `SUNRISE_SPIKES` or while a capture records; the slow ones go to the slow ring, not as span lists to the log.
use std::{
    cell::{Cell, RefCell},
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
const ONGOING: u8 = 1;
const FRAME: u8 = 2;
thread_local! {
    static SPANS: RefCell<Vec<Span>> = const { RefCell::new(Vec::new()) };
    /// Wall-only recording on this thread: the ongoing setting and the scoped frame override.
    static HERE: Cell<u8> = const { Cell::new(0) };
    /// Block-entity load writes and item-render entity visits in this raw-flight frame.
    static BLOCK_ENTITY_WORK: Cell<[u64; 2]> = const { Cell::new([0; 2]) };
    /// How many timed spans the thread is inside: the next span's [`Span::depth`].
    static DEPTH: Cell<u8> = const { Cell::new(0) };
    /// What this thread's slowest steps were busy with since it last took them ([`note`]).
    static NOTES: RefCell<Vec<(f32, String)>> = const { RefCell::new(Vec::new()) };
}

/// The most spans a thread holds before they are taken ([`take_raw`]): a worker thread
/// that records spans (`lod_reduce`, `lt_*`) while they are on for every thread is never
/// taken from, so it drops what it held instead of growing for as long as they stay on.
const UNTAKEN_MAX: usize = 1 << 14;

/// The shortest step a [`note`] keeps, in milliseconds.
pub const NOTE_MS: f32 = 10.0;
/// The notes a thread keeps between two [`take_notes`]: its slowest steps.
const NOTES_KEPT: usize = 3;

/// A named timing: wall milliseconds and the thread's CPU milliseconds in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub name: &'static str,
    pub ms: f32,
    pub cpu_ms: f32,
    /// How many timed spans it ran inside (0: none): `tick_mobs` is inside `tick`.
    pub depth: u8,
}

pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}
/// Whether spans are being recorded.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}
/// Record this thread's spans even with spans off, their wall time only (no CPU clock read):
/// what the render thread's slow frames spent their time in. The thread takes them every
/// frame ([`take`]).
pub fn record_here(on: bool) {
    HERE.with(|h| {
        h.set(if on {
            h.get() | ONGOING
        } else {
            h.get() & !ONGOING
        })
    });
}

/// Record wall time during a measured frame without changing the ongoing setting.
pub fn record_frame() -> FrameRecording {
    let previous = HERE.with(|h| h.replace(h.get() | FRAME) & FRAME);
    if previous == 0 {
        BLOCK_ENTITY_WORK.set([0; 2]);
    }
    FrameRecording {
        previous,
        _thread: PhantomData,
    }
}

#[must_use = "frame recording ends when the guard is dropped"]
pub struct FrameRecording {
    previous: u8,
    _thread: PhantomData<*const ()>,
}

impl FrameRecording {
    /// Load writes and item-render entity visits recorded since the frame started.
    pub fn block_entity_work(&self) -> [u64; 2] {
        BLOCK_ENTITY_WORK.get()
    }
}

/// Time a span only within a scoped recording for a raw flight.
#[inline]
pub fn raw_flight_guard(name: &'static str) -> Option<Guard> {
    (HERE.get() & FRAME != 0).then(|| guard(name))
}

/// Count actual inserts or replacements during a column load, not incoming tag entries.
#[inline]
pub fn note_block_entity_loads(count: usize) {
    if HERE.get() & FRAME != 0 {
        let [loads, probes] = BLOCK_ENTITY_WORK.get();
        BLOCK_ENTITY_WORK.set([loads + count as u64, probes]);
    }
}

/// Count block entities visited in an item-render scan.
#[inline]
pub fn note_block_entity_item_visits(count: impl FnOnce() -> u64) {
    if HERE.get() & FRAME != 0 {
        let [loads, probes] = BLOCK_ENTITY_WORK.get();
        BLOCK_ENTITY_WORK.set([loads, probes + count()]);
    }
}

impl Drop for FrameRecording {
    fn drop(&mut self) {
        HERE.with(|h| h.set((h.get() & !FRAME) | self.previous));
    }
}

/// Whether this thread records spans: globally, here, or for a timeline capture.
#[inline]
pub fn recording_here() -> bool {
    ENABLED.load(Ordering::Relaxed) || timeline::on() || HERE.with(Cell::get) != 0
}
/// The thread's CPU time when spans are on for every thread (`SUNRISE_SPIKES`): spans recorded
/// by `record_here` alone keep wall time only.
#[inline]
fn cpu_if_enabled() -> Option<Duration> {
    if ENABLED.load(Ordering::Relaxed) {
        thread_cpu()
    } else {
        None
    }
}
/// Run `f`, recording its duration under `name` when spans are enabled.
#[inline]
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let _span = guard(name);
    f()
}
/// A span that ends where it is dropped on the opening thread: [`time`] for a step with
/// several ways out (early returns). Nothing is read while spans are off.
#[must_use = "the span ends when the guard is dropped"]
pub struct Guard {
    open: Option<OpenSpan>,
    _heap: Option<sunrise_threads::heap_census::Tag>,
    /// The sampler's span before this one.
    outer: u16,
}

struct OpenSpan {
    name: &'static str,
    started: Instant,
    cpu: Option<Duration>,
    depth: u8,
    timeline: Option<timeline::Scope>,
}

/// Start a span named `name` that ends when the returned [`Guard`] is dropped.
#[inline]
pub fn guard(name: &'static str) -> Guard {
    let _heap = sunrise_threads::heap_tag(name);
    let outer = sunrise_threads::work::enter_span(name);
    let generation = timeline::active_generation();
    let open = (generation.is_some() || enabled() || HERE.with(Cell::get) != 0).then(|| OpenSpan {
        timeline: generation.and_then(timeline::open),
        depth: DEPTH.with(|d| d.replace(d.get().saturating_add(1))),
        name,
        started: Instant::now(),
        cpu: cpu_if_enabled(),
    });
    Guard { open, _heap, outer }
}
impl Drop for Guard {
    fn drop(&mut self) {
        sunrise_threads::work::leave_span(self.outer);
        let Some(OpenSpan {
            name,
            started,
            cpu,
            depth,
            timeline,
        }) = self.open.take()
        else {
            return;
        };
        let elapsed = started.elapsed();
        let ms = elapsed.as_secs_f32() * 1000.0;
        let cpu_ms = cpu
            .and_then(|start| Some((thread_cpu()?, start)))
            .map_or(ms, |(end, start)| (end - start).as_secs_f32() * 1000.0);
        DEPTH.with(|d| d.set(depth));
        if let Some(scope) = timeline {
            scope.finish(name.into(), started, elapsed);
        }
        push(Span {
            name,
            ms,
            cpu_ms,
            depth,
        });
    }
}
/// How many spans this thread holds.
fn recorded() -> usize {
    SPANS.with(|s| s.borrow().len())
}
/// Moves the spans recorded since `mark` one level in: they ran inside the lap now ending.
fn nest_since(mark: usize, depth: u8) {
    SPANS.with(|s| {
        let mut s = s.borrow_mut();
        let start = mark.min(s.len());
        for span in &mut s[start..] {
            if span.depth >= depth {
                span.depth = span.depth.saturating_add(1);
            }
        }
    });
}
/// Keeps a finished span for the thread's next [`take_raw`].
fn push(span: Span) {
    SPANS.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() >= UNTAKEN_MAX {
            s.clear();
        }
        s.push(span);
    });
}
/// Notes that one step inside a span took `ms` milliseconds doing `what` (one player's
/// packets, say), when this thread records spans and the step took at least [`NOTE_MS`]: a
/// span says where a slow tick went, its notes whose doing it was. Only the slowest few are
/// kept, and `what` is built only for a step that is.
pub fn note(ms: f32, what: impl FnOnce() -> String) {
    if ms < NOTE_MS || !recording_here() {
        return;
    }
    NOTES.with(|n| {
        let mut n = n.borrow_mut();
        if n.len() >= NOTES_KEPT {
            let Some((i, _)) = n
                .iter()
                .enumerate()
                .filter(|(_, (kept, _))| *kept < ms)
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            else {
                return;
            };
            n.swap_remove(i);
        }
        n.push((ms, what()));
    });
}
/// This thread's notes ([`note`]), slowest first; clears them.
pub fn take_notes() -> Vec<(f32, String)> {
    let mut notes = NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()));
    notes.sort_by(|a, b| b.0.total_cmp(&a.0));
    notes
}
/// Consecutive steps of one function timed as spans: each [`Laps::lap`] records the time since
/// the previous one (or since [`laps`]) under its name. Nothing is read while spans are off.
pub struct Laps(Option<(Instant, Option<Duration>, usize)>);
/// Start timing the steps of a function (see [`Laps`]).
#[inline]
pub fn laps() -> Laps {
    Laps(recording_here().then(|| (Instant::now(), cpu_if_enabled(), recorded())))
}
impl Laps {
    /// Record the step that just ended under `name`, and start the next.
    #[inline]
    pub fn lap(&mut self, name: &'static str) {
        let Some((started, cpu, mark)) = &mut self.0 else {
            return;
        };
        let now = Instant::now();
        let now_cpu = cpu.and_then(|_| thread_cpu());
        let elapsed = now - *started;
        let ms = elapsed.as_secs_f32() * 1000.0;
        let cpu_ms = now_cpu
            .zip(*cpu)
            .map_or(ms, |(end, start)| (end - start).as_secs_f32() * 1000.0);
        let depth = DEPTH.with(Cell::get);
        timeline::lap(name, *started, elapsed);
        nest_since(*mark, depth);
        push(Span {
            name,
            ms,
            cpu_ms,
            depth,
        });
        (*started, *cpu, *mark) = (now, now_cpu, recorded());
    }
}
/// Sum a repeated phase into one span for the tick breakdown.
#[derive(Clone, Default)]
pub struct SpanTotal {
    wall: Duration,
    cpu: Duration,
    depth: Option<u8>,
}

impl SpanTotal {
    pub fn start() -> Option<(Instant, Option<Duration>)> {
        recording_here().then(|| (Instant::now(), cpu_if_enabled()))
    }

    pub fn add(&mut self, sample: Option<(Instant, Option<Duration>)>) {
        if let Some((started, cpu)) = sample {
            let elapsed = started.elapsed();
            self.wall += elapsed;
            self.cpu += cpu
                .zip(cpu_if_enabled())
                .map_or(elapsed, |(start, end)| end - start);
            self.depth = Some(DEPTH.with(Cell::get));
        }
    }

    pub fn record(self, name: &'static str) {
        if let Some(depth) = self.depth {
            push(Span {
                name,
                ms: self.wall.as_secs_f32() * 1000.0,
                cpu_ms: self.cpu.as_secs_f32() * 1000.0,
                depth,
            });
        }
    }
}

/// The CPU time this thread has used, where the platform tells it: a tick
/// with more wall time than CPU time waited (for a lock, or for a core on a
/// busy machine).
pub fn thread_cpu() -> Option<Duration> {
    #[cfg(unix)]
    {
        let t = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        Some(Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    }
    #[cfg(not(unix))]
    sunrise_threads::thread_cpu()
}

/// The frame's spans as they were recorded, each after the spans inside it; clears them.
pub fn take_raw() -> Vec<Span> {
    SPANS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}
/// The frame's spans, summed by name, slowest first; clears them.
pub fn take() -> Vec<Span> {
    merge(&take_raw())
}
/// What a frame spent its time in, from its spans as recorded ([`take_raw`]): the slowest of
/// the outermost spans (summed by name, as a frame can tick several times), then the slowest
/// span inside it while that one took at least half of it, and so on. The path of names
/// (`tick/tick_mobs`) and the last one's milliseconds; `None` without spans.
pub fn culprit(raw: &[Span]) -> Option<(String, f32)> {
    // The spans directly inside each one: recorded just before it, one level deeper.
    let mut inside: Vec<Vec<usize>> = vec![Vec::new(); raw.len()];
    let mut outer: Vec<usize> = Vec::new();
    for (i, span) in raw.iter().enumerate() {
        while let Some(&j) = outer.last()
            && raw[j].depth > span.depth
        {
            outer.pop();
            inside[i].push(j);
        }
        outer.push(i);
    }
    let slowest = |group: &[usize]| {
        let mut sums: Vec<(&'static str, f32, Vec<usize>)> = Vec::new();
        for &i in group {
            match sums.iter_mut().find(|s| s.0 == raw[i].name) {
                Some(sum) => {
                    sum.1 += raw[i].ms;
                    sum.2.push(i);
                }
                None => sums.push((raw[i].name, raw[i].ms, vec![i])),
            }
        }
        sums.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))
    };
    let (name, mut ms, mut at) = slowest(&outer)?;
    let mut path = name.to_owned();
    loop {
        let next: Vec<usize> = at.iter().flat_map(|&i| inside[i].iter().copied()).collect();
        match slowest(&next) {
            Some((name, inner_ms, spans)) if inner_ms >= ms * 0.5 => {
                path.push('/');
                path.push_str(name);
                (ms, at) = (inner_ms, spans);
            }
            _ => return Some((path, ms)),
        }
    }
}
/// Spans summed by name, slowest first.
pub fn merge(spans: &[Span]) -> Vec<Span> {
    let mut sums: Vec<Span> = Vec::new();
    for &span in spans {
        match sums.iter_mut().find(|s| s.name == span.name) {
            Some(sum) => {
                sum.ms += span.ms;
                sum.cpu_ms += span.cpu_ms;
            }
            None => sums.push(span),
        }
    }
    sums.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    sums
}
/// The run's timeline: every span on every thread, kept in per-thread buffers until a capture drains them as `timeline` chunks.
pub mod timeline {
    use std::{
        borrow::Cow,
        cell::RefCell,
        marker::PhantomData,
        num::NonZeroU64,
        sync::{
            Arc, Mutex, PoisonError,
            atomic::{AtomicU64, Ordering},
        },
        time::{Duration, Instant},
    };

    /// The most spans a thread holds between two drains; a thread that records more drops them.
    const THREAD_EVENTS_MAX: usize = 1 << 18;

    // Odd during a capture, even between captures. Advancing one value under RUN's lock makes
    // the active flag and capture identity change together.
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static RUN: Mutex<Option<Run>> = Mutex::new(None);

    struct Run {
        origin: Instant,
        threads: Vec<Arc<Mutex<Thread>>>,
    }

    struct Thread {
        name: String,
        label: String,
        events: Vec<Event>,
    }

    /// This thread's buffer and open scopes in the timeline it last joined.
    struct Mine {
        generation: u64,
        origin: Instant,
        thread: Arc<Mutex<Thread>>,
        open: Vec<NonZeroU64>,
        next_id: Option<NonZeroU64>,
    }

    thread_local! {
        static MINE: RefCell<Option<Mine>> = const { RefCell::new(None) };
        #[cfg(test)]
        static TEST_GENERATION: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    }

    /// One finished span.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Event {
        pub name: Cow<'static, str>,
        /// Microseconds from the timeline's start.
        pub start_us: u64,
        pub dur_us: u32,
        /// How many spans it ran inside on its thread.
        pub depth: u8,
    }

    /// The spans one thread finished since the last drain, in the order they finished.
    #[derive(Debug, PartialEq, Eq)]
    pub struct ThreadEvents {
        /// The thread's name; threads alive together under one name get `#2`, `#3`... after it.
        pub thread: String,
        pub events: Vec<Event>,
    }

    /// Whether a timeline is being recorded.
    #[inline]
    pub fn on() -> bool {
        active_generation().is_some()
    }

    #[inline]
    pub(super) fn active_generation() -> Option<u64> {
        #[cfg(test)]
        if let Some(generation) = TEST_GENERATION
            .try_with(std::cell::Cell::get)
            .ok()
            .flatten()
        {
            return (generation & 1 == 1).then_some(generation);
        }
        let generation = GENERATION.load(Ordering::Relaxed);
        (generation & 1 == 1).then_some(generation)
    }

    /// Starts recording, timed from `origin`; false when a timeline already runs (the process has one).
    pub fn start(origin: Instant) -> bool {
        let mut run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        if run.is_some() {
            return false;
        }
        *run = Some(Run {
            origin,
            threads: Vec::new(),
        });
        GENERATION.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Takes what the threads finished since the last call.
    pub fn drain() -> Vec<ThreadEvents> {
        let run = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        run.as_ref().map_or_else(Vec::new, take_all)
    }

    /// Stops recording and returns what the threads finished since the last drain.
    pub fn finish() -> Vec<ThreadEvents> {
        let mut slot = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(run) = slot.take() else {
            return Vec::new();
        };
        GENERATION.fetch_add(1, Ordering::Relaxed);
        take_all(&run)
    }

    fn take_all(run: &Run) -> Vec<ThreadEvents> {
        run.threads
            .iter()
            .filter_map(|thread| {
                let mut thread = thread.lock().unwrap_or_else(PoisonError::into_inner);
                (!thread.events.is_empty()).then(|| ThreadEvents {
                    thread: thread.label.clone(),
                    events: std::mem::take(&mut thread.events),
                })
            })
            .collect()
    }

    /// Takes a buffer for this thread: the one of an exited thread of the same name, else a new one.
    fn join(generation: u64) -> Option<Mine> {
        let mut slot = RUN.lock().unwrap_or_else(PoisonError::into_inner);
        if GENERATION.load(Ordering::Relaxed) != generation {
            return None;
        }
        let run = slot.as_mut()?;
        let name = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        let of_name =
            |t: &Arc<Mutex<Thread>>| t.lock().unwrap_or_else(PoisonError::into_inner).name == name;
        let thread = match run
            .threads
            .iter()
            .find(|t| Arc::strong_count(t) == 1 && of_name(t))
        {
            Some(exited) => Arc::clone(exited),
            None => {
                let same = run.threads.iter().filter(|t| of_name(t)).count();
                let label = if same == 0 {
                    name.clone()
                } else {
                    format!("{name} #{}", same + 1)
                };
                let thread = Arc::new(Mutex::new(Thread {
                    name,
                    label,
                    events: Vec::new(),
                }));
                run.threads.push(Arc::clone(&thread));
                thread
            }
        };
        Some(Mine {
            generation,
            origin: run.origin,
            thread,
            open: Vec::new(),
            next_id: Some(NonZeroU64::MIN),
        })
    }

    fn with_buffer<R>(generation: u64, f: impl FnOnce(&mut Mine) -> Option<R>) -> Option<R> {
        MINE.try_with(|mine| {
            let mut mine = mine.try_borrow_mut().ok()?;
            if mine.as_ref().is_none_or(|m| m.generation != generation) {
                *mine = join(generation);
            }
            f(mine.as_mut()?)
        })
        .ok()
        .flatten()
    }

    /// An open scope's identity on its thread. Depth alone would alias a new scope after a
    /// malformed suffix was discarded.
    pub(super) struct Scope {
        id: NonZeroU64,
        generation: u64,
        depth: usize,
        _thread: PhantomData<*const ()>,
    }

    pub(super) fn open(generation: u64) -> Option<Scope> {
        with_buffer(generation, |mine| {
            let id = mine.next_id?;
            mine.next_id = id.checked_add(1);
            let depth = mine.open.len();
            mine.open.push(id);
            Some(Scope {
                id,
                generation,
                depth,
                _thread: PhantomData,
            })
        })
    }

    impl Scope {
        pub(super) fn finish(self, name: Cow<'static, str>, started: Instant, elapsed: Duration) {
            if active_generation() != Some(self.generation) {
                return;
            }
            // A scope belongs to the buffer that opened it. Do not join (or replace a buffer)
            // on drop, including after the thread's recorder has already been torn down.
            let _ = MINE.try_with(|mine| {
                let Ok(mut mine) = mine.try_borrow_mut() else {
                    return;
                };
                let Some(mine) = mine.as_mut() else {
                    return;
                };
                if mine.generation != self.generation || mine.open.get(self.depth) != Some(&self.id)
                {
                    return;
                }
                let top = self.depth + 1 == mine.open.len();
                // A non-top close discards the malformed open suffix, leaving its parent alone.
                mine.open.truncate(self.depth);
                if top {
                    mine.push(name, started, elapsed);
                }
            });
        }
    }

    impl Mine {
        fn push(&self, name: Cow<'static, str>, started: Instant, elapsed: Duration) {
            let mut thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
            // Recheck under the buffer lock so a finishing capture cannot drain this buffer and
            // then receive another event from a scope that was already open.
            if active_generation() != Some(self.generation)
                || thread.events.len() >= THREAD_EVENTS_MAX
            {
                return;
            }
            // Quantize both endpoints on the same clock so nested endpoints stay ordered.
            let start = started.saturating_duration_since(self.origin);
            let start_us = start.as_micros();
            let end_us = start.saturating_add(elapsed).as_micros();
            thread.events.push(Event {
                name,
                start_us: start_us as u64,
                dur_us: (end_us - start_us).min(u128::from(u32::MAX)) as u32,
                depth: u8::try_from(self.open.len()).unwrap_or(u8::MAX),
            });
        }
    }

    /// Open a mod's named zone in the current timeline. The returned action closes it on the
    /// same thread; when no timeline is active there is no allocation or clock read.
    pub fn named_zone(label: &str) -> Option<Box<dyn FnOnce()>> {
        let scope = open(active_generation()?)?;
        let name = Cow::Owned(label.to_owned());
        let started = Instant::now();
        Some(Box::new(move || {
            scope.finish(name, started, started.elapsed())
        }))
    }

    /// Record a finished lap at the current timeline depth, separate from raw span depth.
    pub(super) fn lap(name: &'static str, started: Instant, elapsed: Duration) {
        if let Some(generation) = active_generation() {
            with_buffer(generation, |mine| {
                mine.push(name.into(), started, elapsed);
                Some(())
            });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use mod_platform::{profiling::zone, testing::with_named_zone_recorder};

        // Replace just this thread's timeline state, without touching the process recorder.
        fn with_session<R>(generation: u64, mine: Option<Mine>, f: impl FnOnce() -> R) -> R {
            struct Restore(Option<u64>, Option<Mine>);
            impl Drop for Restore {
                fn drop(&mut self) {
                    TEST_GENERATION.set(self.0);
                    MINE.set(self.1.take());
                }
            }
            let _restore = Restore(
                TEST_GENERATION.replace(Some(generation)),
                MINE.replace(mine),
            );
            f()
        }

        fn capture<R>(generation: u64, f: impl FnOnce() -> R) -> (R, Vec<Event>) {
            let thread = Arc::new(Mutex::new(Thread {
                name: "test".into(),
                label: "test".into(),
                events: Vec::new(),
            }));
            let mine = Mine {
                generation,
                origin: Instant::now(),
                thread: Arc::clone(&thread),
                open: Vec::new(),
                next_id: Some(NonZeroU64::MIN),
            };
            let result = with_session(generation, Some(mine), f);
            let events = std::mem::take(&mut thread.lock().unwrap().events);
            (result, events)
        }

        fn names_and_depths(events: &[Event]) -> Vec<(&str, u8)> {
            events.iter().map(|e| (e.name.as_ref(), e.depth)).collect()
        }

        #[test]
        fn named_mod_zones_nest_with_engine_spans_and_close_on_return() {
            fn work(early: bool) -> bool {
                let _work = zone("mod-work");
                let _stage = zone(&format!("mod-stage-{}", 'β'));
                if early {
                    return false;
                }
                true
            }

            let ((), events) = capture(1, || {
                drop(zone("no recorder in standalone tests"));
                with_named_zone_recorder(named_zone, || {
                    let _engine = crate::spans::guard("engine");
                    assert!(!work(true));
                    let _after_return = zone("after-return");
                });
            });
            assert_eq!(
                names_and_depths(&events),
                [
                    ("mod-stage-β", 2),
                    ("mod-work", 1),
                    ("after-return", 1),
                    ("engine", 0)
                ]
            );
            let inner = &events[0];
            let outer = &events[1];
            assert!(inner.start_us >= outer.start_us);
            assert!(
                inner.start_us + u64::from(inner.dur_us)
                    <= outer.start_us + u64::from(outer.dur_us)
            );
        }

        #[test]
        fn inactive_and_stale_zones_do_not_record_in_a_new_capture() {
            with_named_zone_recorder(named_zone, || {
                let inactive = with_session(0, None, || zone("off"));
                let (stale, first_events) = capture(1, || zone("first capture"));
                assert!(first_events.is_empty());
                let ((), events) = capture(3, || {
                    let _new = zone("next capture");
                    drop(stale);
                    drop(inactive);
                });
                assert_eq!(names_and_depths(&events), [("next capture", 0)]);
            });
        }

        #[test]
        fn closing_a_zone_out_of_order_does_not_close_a_later_zone_at_the_same_depth() {
            let ((), events) = with_named_zone_recorder(named_zone, || {
                capture(1, || {
                    let old_root = zone("old root");
                    let old_child = zone("old child");
                    drop(old_root);
                    let _new_root = zone("new root");
                    let _new_child = zone("new child");
                    drop(old_child);
                })
            });
            assert_eq!(
                names_and_depths(&events),
                [("new child", 1), ("new root", 0)]
            );
        }

        #[test]
        fn an_engine_lap_uses_the_current_timeline_depth() {
            let ((), events) = with_named_zone_recorder(named_zone, || {
                capture(1, || {
                    let _ = crate::spans::take_raw();
                    let _mod_scope = zone("mod scope");
                    crate::spans::laps().lap("engine lap");
                    let raw = crate::spans::take_raw();
                    assert_eq!(
                        raw.iter().map(|s| (s.name, s.depth)).collect::<Vec<_>>(),
                        [("engine lap", 0)]
                    );
                })
            });
            assert_eq!(
                names_and_depths(&events),
                [("engine lap", 1), ("mod scope", 0)]
            );
        }
    }
}

/// The start's timeline: `SUNRISE_STARTUP_TRACE=FILE` writes every named step of
/// the start (resource tables, the renderer, the title, a world's open, a
/// server's join, a dimension change) on whichever thread ran it, one Chrome
/// trace event per line (`tools/startup_trace.py` reads it; wrapped in `[...]`,
/// Perfetto opens it). Times are microseconds since the process started. Off,
/// a step costs one relaxed load.
pub mod startup {
    use std::{
        io::Write as _,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicBool, AtomicU32, Ordering},
        },
        time::Instant,
    };

    static ON: AtomicBool = AtomicBool::new(false);
    static SINK: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    static NEXT_THREAD: AtomicU32 = AtomicU32::new(1);
    thread_local! {
        /// This thread's trace id, given (with its name) on its first event.
        static THREAD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// Opens the trace named by `SUNRISE_STARTUP_TRACE`, timed from `origin`
    /// (the process's start). Call once, first thing.
    pub fn init(origin: Instant) {
        ORIGIN.get_or_init(|| origin);
        let sink = SINK.get_or_init(|| {
            let path = std::env::var_os("SUNRISE_STARTUP_TRACE")?;
            std::fs::File::create(&path).ok().map(Mutex::new)
        });
        ON.store(sink.is_some(), Ordering::Relaxed);
    }

    /// Whether a trace is being written.
    #[inline]
    pub fn on() -> bool {
        ON.load(Ordering::Relaxed)
    }

    fn micros(at: Instant) -> u128 {
        ORIGIN.get().map_or(0, |origin| {
            at.saturating_duration_since(*origin).as_micros()
        })
    }

    fn write(event: std::fmt::Arguments) {
        let Some(Some(sink)) = SINK.get() else {
            return;
        };
        let tid = THREAD.with(|id| {
            if id.get() == 0 {
                id.set(NEXT_THREAD.fetch_add(1, Ordering::Relaxed));
                let name = std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .replace(['"', '\\'], "_");
                let mut file = sink.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = writeln!(
                    file,
                    r#"{{"ph":"M","name":"thread_name","pid":1,"tid":{},"args":{{"name":"{name}"}}}}"#,
                    id.get()
                );
            }
            id.get()
        });
        let mut file = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = writeln!(file, r#"{{"pid":1,"tid":{tid},{event}}}"#);
    }

    /// A step running until it is dropped.
    #[must_use = "the step ends when this is dropped"]
    pub struct Step {
        name: &'static str,
        started: Instant,
        traced: bool,
        /// A heap-census build's tag for the step's allocations.
        _heap: Option<sunrise_threads::heap_census::Tag>,
    }
    impl Drop for Step {
        fn drop(&mut self) {
            if !self.traced {
                return;
            }
            let end = Instant::now();
            write(format_args!(
                r#""ph":"X","name":"{}","ts":{},"dur":{}"#,
                self.name,
                micros(self.started),
                end.saturating_duration_since(self.started).as_micros()
            ));
        }
    }

    /// Starts a step named `name` (a literal: no quotes or backslashes).
    #[inline]
    pub fn step(name: &'static str) -> Option<Step> {
        let traced = on();
        (traced || sunrise_threads::HEAP_CENSUS).then(|| Step {
            name,
            started: Instant::now(),
            traced,
            _heap: sunrise_threads::heap_tag(name),
        })
    }

    /// Runs `f` as a step named `name`.
    #[inline]
    pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
        let _step = step(name);
        f()
    }

    /// A moment of the start: the title drawn, the world in view.
    #[inline]
    pub fn mark(name: &'static str) {
        if on() {
            write(format_args!(
                r#""ph":"i","s":"g","name":"{name}","ts":{}"#,
                micros(Instant::now())
            ));
        }
    }

    /// Set while a world opens, a server is joined or the dimension changes,
    /// until the view first fills ([`view_filled`]).
    static FILLING: AtomicBool = AtomicBool::new(false);

    /// A world starts loading: the next full view is marked.
    pub fn arm(name: &'static str) {
        if on() {
            mark(name);
            FILLING.store(true, Ordering::Relaxed);
        }
    }

    /// Whether a world's view is still filling (between [`arm`] and the
    /// `world.view_full` mark).
    #[inline]
    pub fn filling() -> bool {
        on() && FILLING.load(Ordering::Relaxed)
    }

    /// Values at this moment, as a counter track named `name` (Perfetto draws
    /// each value as a line): what the view waits on while it fills.
    pub fn counters(name: &'static str, values: &[(&'static str, usize)]) {
        if !on() {
            return;
        }
        let args: Vec<String> = values
            .iter()
            .map(|(k, v)| format!(r#""{k}":{v}"#))
            .collect();
        write(format_args!(
            r#""ph":"C","name":"{name}","ts":{},"args":{{{}}}"#,
            micros(Instant::now()),
            args.join(",")
        ));
    }

    /// Called each frame in a world: marks `world.view_full` the first frame
    /// after [`arm`] where nothing waits to load, generate, light, mesh or
    /// upload and the resident columns cover perf_bench's share of the view
    /// (0.8 pi r^2: corners are never loaded).
    #[inline]
    pub fn view_filled(resident: usize, pending: usize, radius: u32) {
        if FILLING.load(Ordering::Relaxed)
            && pending == 0
            && resident as f64 >= 0.8 * std::f64::consts::PI * f64::from(radius * radius)
            && FILLING.swap(false, Ordering::Relaxed)
        {
            mark("world.view_full");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Span;
    use std::sync::{Mutex, MutexGuard};

    /// Spans on for every thread ([`super::enable`]) is process-wide: a test that turns them on
    /// would make another, run beside it, record what it expects left out. Those tests take
    /// this lock.
    fn spans_switch() -> MutexGuard<'static, ()> {
        static SWITCH: Mutex<()> = Mutex::new(());
        SWITCH.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Samples taken inside a named span carry its id, with spans off, and the span is left after it.
    #[cfg(target_os = "linux")]
    #[test]
    fn samples_inside_a_span_carry_its_id() {
        use sunrise_threads::sampler::test_support::{ensure, first_passing_run};
        use sunrise_threads::{sampler, work};
        let minimum = sampler::test_support::cpu_count(30) as usize;
        // A loaded runner can starve one run of its samples: judged on the first of five runs
        // whose checks all hold. A span left on, or samples filed under it outside, fail every run.
        first_passing_run(|| {
            let worker = std::thread::spawn(|| {
                assert!(sampler::sampled_thread("spans-id-test"));
                let config = sampler::Config {
                    hz: 1000,
                    ..Default::default()
                };
                let session = sampler::start(config, |n| n == "spans-id-test").unwrap();
                // Match calibration's CPU-time denominator, not scheduler-dependent wall time.
                let busy = |ms: u64| {
                    let until = sampler::thread_cpu_ns() + ms * 1_000_000;
                    while sampler::thread_cpu_ns() < until {
                        std::hint::black_box(0);
                    }
                };
                super::time("spans_id_test_named", || busy(60));
                assert_eq!(sampler::span(), 0);
                busy(60);
                let mut spans = Vec::new();
                session.stop(|_, r| spans.push(r.ctx.span));
                spans
            });
            let spans = worker.join().unwrap();
            let inside = |s: &u16| work::name(work::Space::Span, *s) == Some("spans_id_test_named");
            let (named, plain) = (
                spans.iter().filter(|s| inside(s)).count(),
                spans.iter().filter(|s| **s == 0).count(),
            );
            ensure(named > minimum && plain > minimum, || {
                format!(
                    "{named} named, {plain} outside of {} samples (minimum {minimum} each)",
                    spans.len()
                )
            })?;
            ensure(named + plain == spans.len(), || "another span id".into())?;
            // Equal CPU work should split attribution in half. At 250 Hz each 60 ms
            // section has about 15 samples; 0.15 allows a few boundary samples, but
            // rejects a substantial attribution bias without changing with the rate.
            let named_share = named as f64 / (named + plain) as f64;
            ensure((named_share - 0.5).abs() < 0.15, || {
                format!("{named} named, {plain} outside")
            })
        });
    }

    /// A slow tick's notes are its slowest steps, slowest first; short steps and steps on a
    /// thread that records no spans leave none.
    #[test]
    fn notes_keep_the_slowest_steps() {
        let _switch = spans_switch();
        super::record_here(false);
        super::note(500.0, || "unrecorded".into());
        assert!(super::take_notes().is_empty());
        super::record_here(true);
        for (ms, what) in [
            (12.0, "a"),
            (900.0, "b"),
            (3.0, "short"),
            (40.0, "c"),
            (11.0, "d"),
            (60.0, "e"),
        ] {
            super::note(ms, || what.into());
        }
        let notes: Vec<(f32, String)> = super::take_notes();
        super::record_here(false);
        let names: Vec<&str> = notes.iter().map(|(_, w)| w.as_str()).collect();
        assert_eq!(names, ["b", "e", "c"]);
        assert!(super::take_notes().is_empty(), "taking them clears them");
    }

    #[test]
    fn recording_one_frame_does_not_change_ongoing_recording() {
        let _switch = spans_switch();
        super::time("before", || ());
        {
            let _frame = super::record_frame();
            super::time("frame", || ());
        }
        super::time("after", || ());
        {
            let _frame = super::record_frame();
            super::record_here(true);
        }
        super::time("ongoing", || ());
        super::record_here(false);
        let names: Vec<_> = super::take_raw()
            .into_iter()
            .map(|span| span.name)
            .collect();
        assert_eq!(names, ["frame", "ongoing"]);
    }

    #[test]
    fn block_entity_work_and_spans_are_scoped_to_raw_flight_frames() {
        let _switch = spans_switch();
        super::record_here(true);
        assert!(super::raw_flight_guard("outside_flight").is_none());
        super::note_block_entity_loads(19);
        super::note_block_entity_item_visits(|| panic!("must not count outside a raw flight"));
        {
            let frame = super::record_frame();
            {
                let _load = super::raw_flight_guard("load_block_entities");
                super::note_block_entity_loads(1);
                super::note_block_entity_loads(3);
            }
            super::note_block_entity_item_visits(|| 64);
            super::note_block_entity_item_visits(|| 81);
            assert_eq!(frame.block_entity_work(), [4, 145]);
        }
        super::record_here(false);
        assert_eq!(super::record_frame().block_entity_work(), [0; 2]);
        let names: Vec<_> = super::take_raw()
            .into_iter()
            .map(|span| span.name)
            .collect();
        assert_eq!(names, ["load_block_entities"]);
    }

    /// Spans of one name add up, the slowest first; a span that waited keeps its small CPU time.
    #[test]
    fn spans_sum_by_name_and_show_waits() {
        super::SPANS.with(|s| {
            s.borrow_mut().extend([
                Span {
                    name: "tick",
                    ms: 2.0,
                    cpu_ms: 2.0,
                    depth: 0,
                },
                Span {
                    name: "submit",
                    ms: 30.0,
                    cpu_ms: 0.2,
                    depth: 0,
                },
                Span {
                    name: "tick",
                    ms: 3.0,
                    cpu_ms: 2.9,
                    depth: 0,
                },
                Span {
                    name: "hud",
                    ms: 0.3,
                    cpu_ms: 0.3,
                    depth: 0,
                },
            ]);
        });
        let spans = super::take();
        assert_eq!(
            spans[0],
            Span {
                name: "submit",
                ms: 30.0,
                cpu_ms: 0.2,
                depth: 0,
            }
        );
        assert_eq!((spans[1].name, spans[1].ms), ("tick", 5.0));
        assert!(super::take().is_empty());
    }

    /// A thread that records on its own keeps its spans with spans off, each with how many
    /// spans it ran inside, after them.
    #[test]
    fn a_recording_thread_keeps_its_spans_and_their_nesting() {
        std::thread::spawn(|| {
            super::record_here(true);
            super::time("tick", || {
                super::time("tick_mobs", || ());
                super::time("tick_items", || ());
            });
            super::time("renderer", || ());
            let raw = super::take_raw();
            let seen: Vec<_> = raw.iter().map(|s| (s.name, s.depth)).collect();
            assert_eq!(
                seen,
                [
                    ("tick_mobs", 1),
                    ("tick_items", 1),
                    ("tick", 0),
                    ("renderer", 0)
                ]
            );
        })
        .join()
        .unwrap();
    }

    /// A guarded span ends on every way out of its step, an early return's too, and holds the
    /// spans that ran inside it until then; the next span outside it is back at its depth.
    #[test]
    fn a_guarded_span_ends_where_its_step_returns() {
        fn step(early: bool) -> u32 {
            let _span = super::guard("use_block");
            super::time("open_loot", || ());
            if early {
                return 1;
            }
            super::time("open_maps", || ());
            2
        }
        std::thread::spawn(|| {
            super::record_here(true);
            assert_eq!(step(true), 1);
            assert_eq!(step(false), 2);
            super::time("use_item", || ());
            let raw = super::take_raw();
            let seen: Vec<_> = raw.iter().map(|s| (s.name, s.depth)).collect();
            assert_eq!(
                seen,
                [
                    ("open_loot", 1),
                    ("use_block", 0),
                    ("open_loot", 1),
                    ("open_maps", 1),
                    ("use_block", 0),
                    ("use_item", 0)
                ]
            );
        })
        .join()
        .unwrap();
    }

    /// A slow frame is put down to the slowest outer span, then to the span inside it that took
    /// at least half of it; repeated spans (several ticks a frame) add up.
    #[test]
    fn a_frame_is_put_down_to_the_step_that_took_most_of_it() {
        let span = |name, ms, depth| Span {
            name,
            ms,
            cpu_ms: ms,
            depth,
        };
        let frame = [
            span("tick_mobs", 90.0, 1),
            span("tick_items", 5.0, 1),
            span("tick", 100.0, 0),
            span("tick_mobs", 90.0, 1),
            span("tick", 100.0, 0),
            span("r_uploads", 120.0, 1),
            span("renderer", 150.0, 0),
        ];
        assert_eq!(
            super::culprit(&frame),
            Some(("tick/tick_mobs".to_owned(), 180.0))
        );
        // Spread over many steps: the outer span alone.
        let spread = [
            span("tick_mobs", 40.0, 1),
            span("tick_items", 40.0, 1),
            span("tick", 300.0, 0),
        ];
        assert_eq!(super::culprit(&spread), Some(("tick".to_owned(), 300.0)));
        assert_eq!(super::culprit(&[]), None);
    }

    /// Laps record the steps between them while spans are on, and nothing while off.
    #[test]
    fn laps_time_consecutive_steps() {
        let _switch = spans_switch();
        let _ = super::take();
        let mut off = super::laps();
        off.lap("unseen");
        super::enable(true);
        let mut laps = super::laps();
        std::thread::sleep(std::time::Duration::from_millis(2));
        laps.lap("first");
        laps.lap("second");
        super::enable(false);
        off.lap("still unseen");
        let spans = super::take();
        let names: Vec<_> = spans.iter().map(|s| s.name).collect();
        assert_eq!(names, ["first", "second"]);
        assert!(spans[0].ms >= 2.0);
    }

    /// A span that ran during a lap belongs inside that lap, and not inside the next one.
    #[test]
    fn a_span_during_a_lap_nests_under_that_lap() {
        let _switch = spans_switch();
        let _ = super::take_raw();
        super::enable(true);
        let mut laps = super::laps();
        super::time("inner", || {});
        laps.lap("first");
        laps.lap("second");
        super::enable(false);
        let spans = super::take_raw();
        let depths: Vec<_> = spans.iter().map(|s| (s.name, s.depth)).collect();
        assert_eq!(depths, [("inner", 1), ("first", 0), ("second", 0)]);
    }

    /// The thread's CPU clock moves with work done on it, not with time
    /// spent asleep.
    #[cfg(any(unix, all(windows, target_arch = "x86_64")))]
    #[test]
    fn thread_cpu_counts_work_not_sleep() {
        let start = super::thread_cpu().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let slept = super::thread_cpu().unwrap() - start;
        // Busy until the clock shows 10 ms of work (a loaded machine gives
        // the thread less than its wall time).
        let mut x = 0u64;
        let busy = std::time::Instant::now();
        let worked = loop {
            x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
            let worked = super::thread_cpu().unwrap() - start - slept;
            if worked.as_millis() >= 10 || busy.elapsed().as_secs() >= 10 {
                break worked;
            }
        };
        assert!(slept.as_millis() < 25, "{slept:?} while asleep");
        assert!(worked.as_millis() >= 10, "{worked:?} while busy");
    }

    /// Each thread's finished spans reach the timeline with their depth and nesting in time; threads alive together under one name get labels of their own, ones after another share a lane.
    #[test]
    fn the_timeline_keeps_each_threads_spans_with_their_depth_and_place_in_time() {
        use super::timeline;
        use std::sync::{Arc, Barrier};
        use std::time::{Duration, Instant};
        let _switch = spans_switch();
        assert!(timeline::start(Instant::now()));
        assert!(
            !timeline::start(Instant::now()),
            "the process has one timeline"
        );
        let spawn = |name: &str, alive_with: Option<Arc<Barrier>>| {
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    super::time("outer", || {
                        std::thread::sleep(Duration::from_millis(4));
                        super::time("inner", || std::thread::sleep(Duration::from_millis(3)));
                    });
                    if let Some(barrier) = alive_with {
                        barrier.wait();
                    }
                })
                .unwrap()
        };
        spawn("tl-solo", None).join().unwrap();
        let together = Arc::new(Barrier::new(2));
        let twins = [
            spawn("tl-twin", Some(together.clone())),
            spawn("tl-twin", Some(together)),
        ];
        for twin in twins {
            twin.join().unwrap();
        }
        spawn("tl-seq", None).join().unwrap();
        spawn("tl-seq", None).join().unwrap();
        let mut threads = timeline::finish();
        threads.retain(|t| t.thread.starts_with("tl-"));
        threads.sort_by(|a, b| a.thread.cmp(&b.thread));
        let labels: Vec<_> = threads.iter().map(|t| t.thread.as_str()).collect();
        assert_eq!(labels, ["tl-seq", "tl-solo", "tl-twin", "tl-twin #2"]);
        for thread in &threads {
            let runs = if thread.thread == "tl-seq" { 2 } else { 1 };
            assert_eq!(thread.events.len(), 2 * runs, "{thread:?}");
            for pair in thread.events.chunks(2) {
                let [inner, outer] = pair else { unreachable!() };
                assert_eq!((inner.name.as_ref(), inner.depth), ("inner", 1));
                assert_eq!((outer.name.as_ref(), outer.depth), ("outer", 0));
                assert!(
                    inner.dur_us >= 3_000 && outer.dur_us >= 7_000,
                    "{inner:?} {outer:?}"
                );
                assert!(inner.start_us >= outer.start_us + 4_000);
                assert!(
                    inner.start_us + u64::from(inner.dur_us)
                        <= outer.start_us + u64::from(outer.dur_us)
                );
            }
        }
        let seq = threads.iter().find(|t| t.thread == "tl-seq").unwrap();
        assert!(seq.events[2].start_us >= seq.events[1].start_us + u64::from(seq.events[1].dur_us));

        spawn("tl-after", None).join().unwrap();
        assert!(
            timeline::drain().is_empty(),
            "a stopped timeline records nothing"
        );
    }
}
