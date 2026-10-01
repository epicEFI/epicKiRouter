//! The M5-T1 allocation-cost instrument (measurement-only, harness-side).
//!
//! A std-only counting `#[global_allocator]`, compiled ONLY under the
//! `alloc-profile` feature: default builds cfg the static out entirely and
//! keep the std `System` allocator — the production binary face is
//! untouched. The engine crates (`rust/crates/epic-*`) are NOT modified by
//! this module; the instrument is process-wide but armed only by the
//! `alloc-route` command (see [`crate::alloc_route`]).
//!
//! What it measures, once [`install`] arms it:
//!
//! * **Totals** — allocation count + bytes, deallocation count + bytes,
//!   live bytes, peak live bytes (saturating: allocations made BEFORE
//!   [`install`] are uncounted, so a post-install free of one floors `live`
//!   at zero rather than underflowing; the pre-install population is
//!   process-startup scale — kilobytes — against multi-GB route churn).
//!   The mirror asymmetry: DEALLOCS/DEALLOC_BYTES can legitimately EXCEED
//!   ALLOCS/ALLOC_BYTES by the same pre-install population (its frees are
//!   counted while its allocations are not) — not an inconsistency.
//! * **The 1-second bucket histogram** — per-second allocation count +
//!   bytes since install, the phase-attribution face (fanout → batch
//!   passes → optimizer are aligned against the route's own stderr stage
//!   lines by elapsed time).
//! * **Sampled backtrace sites** — `--sample N`: every Nth allocation
//!   captures `std::backtrace::Backtrace::force_capture()`, resolves it,
//!   and aggregates count+bytes per unique trace. This is the RANKED
//!   top-sites face. Sampling requires the instrument binary to carry
//!   release debuginfo (build with
//!   `CARGO_PROFILE_RELEASE_DEBUG=2 --features alloc-profile`) or the
//!   resolved frames are opaque addresses.
//!
//! Re-entrancy: a capture or a stats flush allocates, so allocations made
//! while the thread-local guard is set route straight to `System`
//! uncounted and unsampled — the instrument can never recurse into itself.
//! The guards' own allocations are excluded from the totals symmetrically.
//!
//! Overhead accounting: the unsampled counting path is a handful of
//! relaxed atomics per allocation; the sampled path adds one full
//! capture+symbolize per N allocations (measured against the unsampled
//! twin run — see the M5-T1 report's overhead table).

#![allow(unsafe_code)] // a GlobalAlloc impl is unsafe by definition; the
// ONLY unsafe operations here are System's own alloc/dealloc forwarding.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// One-second buckets, a 6 h ring. The u64 per-bucket counters cannot
/// saturate in practice (at 364 MB/s that takes centuries); the operative
/// bound is the ring itself — allocations past 6 h fall off it (the
/// saturating index drop), never a panic.
const BUCKETS: usize = 21_600;

/// Top-N sites written into the stats file (the full map stays in memory).
const TOP_SITES: usize = 50;

/// Site-map entry cap: once the map reaches it, samples of NEW distinct
/// traces are dropped AND already-recorded sites stop aggregating — both
/// counted in `SITE_CAP_DROPS` (rendered `distinct_cap_drops`) — rather
/// than growing memory without bound.
const SITES_CAP: usize = 50_000;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOCS: AtomicU64 = AtomicU64::new(0);
static DEALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static SAMPLE_EVERY: AtomicU64 = AtomicU64::new(0);
static SAMPLE_SEQ: AtomicU64 = AtomicU64::new(0);
static SAMPLED_ALLOCS: AtomicU64 = AtomicU64::new(0);
static SAMPLED_BYTES: AtomicU64 = AtomicU64::new(0);
static SITE_CAP_DROPS: AtomicU64 = AtomicU64::new(0);

static BUCKET_ALLOCS: [AtomicU64; BUCKETS] = [const { AtomicU64::new(0) }; BUCKETS];
static BUCKET_BYTES: [AtomicU64; BUCKETS] = [const { AtomicU64::new(0) }; BUCKETS];

static T0: OnceLock<Instant> = OnceLock::new();
/// Switchable so [`re_arm`] can point the SECOND attribution window at
/// its own stats file (M6-T1b); read once per flush, never in the alloc
/// hot path, so the mutex costs nothing measurable.
static STATS_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static SITES: Mutex<Option<HashMap<String, (u64, u64)>>> = Mutex::new(None);
/// Serializes the write-then-rename critical section of the TWO writers
/// (the 1 s watcher thread and [`freeze`]) so their tmp-file contents
/// cannot interleave (quality-review MINOR-2: the mutex, not
/// thread-unique tmp names, because one tmp path strands no orphan tmp
/// files on a kill between write and rename and the ordering it gives is
/// trivially provable — whole critical section excluded).
static STATS_WRITE_LOCK: Mutex<()> = Mutex::new(());
/// One-time warning latch for the first stats-write failure (MINOR-5).
static STATS_WRITE_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set while the current thread runs instrument-internal work (a
    /// backtrace capture, a stats flush): allocations made inside route
    /// straight to `System` uncounted, so the instrument cannot recurse.
    static IN_INSTRUMENT: Cell<bool> = const { Cell::new(false) };
}

/// The feature build's process-wide allocator. Dormant (a single relaxed
/// load per allocation) until [`install`] arms it.
#[global_allocator]
static GLOBAL_ALLOC: CountingAlloc = CountingAlloc;

/// The counting allocator: forwards EVERY operation to [`System`] first —
/// counting never blocks or fails an allocation — then records.
struct CountingAlloc;

// GlobalAlloc is an unsafe trait; the bodies' only unsafe operations are
// the System forwards, each in its own unsafe block (edition 2024's
// unsafe_op_in_unsafe_fn).
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && ARMED.load(Ordering::Relaxed) {
            self.count_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            self.count_dealloc(layout.size());
        }
        unsafe { System.dealloc(ptr, layout) };
    }

    // The zeroed/realloc overrides forward DIRECTLY to System so each
    // operation is counted exactly once (the trait defaults would compose
    // alloc/dealloc and double-count through this allocator's own hooks).
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && ARMED.load(Ordering::Relaxed) {
            self.count_alloc(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, old_layout, new_size) };
        if !new_ptr.is_null() && ARMED.load(Ordering::Relaxed) {
            self.count_dealloc(old_layout.size());
            self.count_alloc(new_size);
        }
        new_ptr
    }
}

impl CountingAlloc {
    fn count_alloc(&self, size: usize) {
        if IN_INSTRUMENT.get() {
            return;
        }
        let size = size as u64;
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(size, Ordering::Relaxed);
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(live, Ordering::Relaxed);
        if let Some(t0) = T0.get() {
            let idx = t0.elapsed().as_secs() as usize;
            if idx < BUCKETS {
                BUCKET_ALLOCS[idx].fetch_add(1, Ordering::Relaxed);
                BUCKET_BYTES[idx].fetch_add(size, Ordering::Relaxed);
            }
        }
        let every = SAMPLE_EVERY.load(Ordering::Relaxed);
        if every > 0 {
            let seq = SAMPLE_SEQ.fetch_add(1, Ordering::Relaxed);
            if seq.is_multiple_of(every) {
                self.record_sample(size);
            }
        }
    }

    fn count_dealloc(&self, size: usize) {
        if IN_INSTRUMENT.get() {
            return;
        }
        let size = size as u64;
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        DEALLOC_BYTES.fetch_add(size, Ordering::Relaxed);
        // Saturating decrement (see the module doc): a pre-install
        // allocation freed post-install must not underflow `live`.
        loop {
            let live = LIVE.load(Ordering::Relaxed);
            let next = live.saturating_sub(size);
            if LIVE
                .compare_exchange_weak(live, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    fn record_sample(&self, bytes: u64) {
        // The caller (count_alloc) has already returned when the
        // thread-local guard is set, so this body only ever runs on an
        // unguarded thread; the guard is set HERE around the capture.
        IN_INSTRUMENT.with(|flag| flag.set(true));
        let key = format!("{}", std::backtrace::Backtrace::force_capture());
        if let Ok(mut sites) = SITES.lock()
            && let Some(map) = sites.as_mut()
        {
            if map.len() < SITES_CAP {
                let entry = map.entry(key).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += bytes;
            } else {
                SITE_CAP_DROPS.fetch_add(1, Ordering::Relaxed);
            }
        }
        SAMPLED_ALLOCS.fetch_add(1, Ordering::Relaxed);
        SAMPLED_BYTES.fetch_add(bytes, Ordering::Relaxed);
        IN_INSTRUMENT.with(|flag| flag.set(false));
    }
}

/// One instant's counter face (the delta between two snapshots is the
/// attribution window).
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub allocs: u64,
    pub alloc_bytes: u64,
    pub live: u64,
    pub peak: u64,
}

/// Arms the instrument: counters on, 1 s flush watcher spawned. Called by
/// `alloc-route` before the fixture pipeline is built.
pub fn install(stats_path: &Path, sample_every: u64) {
    let _ = T0.set(Instant::now());
    if let Ok(mut slot) = STATS_PATH.lock() {
        *slot = Some(stats_path.to_path_buf());
    }
    SAMPLE_EVERY.store(sample_every, Ordering::Relaxed);
    if sample_every > 0 {
        let mut sites = SITES.lock().expect("sites mutex poisoned at install");
        *sites = Some(HashMap::new());
    }
    ARMED.store(true, Ordering::Relaxed);
    // The watcher keeps the stats file current on a 1 s cadence so an
    // externally-killed run (the bm11 face) still delivers its counts.
    std::thread::Builder::new()
        .name("alloc-stats-flush".to_string())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                write_stats_file();
            }
        })
        .expect("alloc-stats flush thread spawn");
}

/// Unarms the instrument and writes the final stats file: everything
/// allocated AFTER this call is uncounted, so the file's totals are the
/// FREEZE-TIME face — setup (parse/board/normalize/DRC seed/settings,
/// armed since [`install`]) plus the attribution window; only the
/// post-route face walks are excluded.
pub fn freeze() {
    ARMED.store(false, Ordering::Relaxed);
    write_stats_file();
}

/// Re-arms the instrument for a SECOND attribution window (the M6-T1b
/// optimizer-window face): switches the stats path, resets the
/// window-relative counters and the sampled-sites map, and leaves the
/// global timeline (T0/buckets) and the process-wide `live`/`peak` faces
/// untouched — the second file's nonzero buckets therefore bracket the
/// window directly, and `live` keeps its saturating-decrement
/// correctness. Call only AFTER [`freeze`] (the first window's file is
/// already final); the 1 s watcher keeps flushing, now to the new path.
pub fn re_arm(stats_path: &Path, sample_every: u64) {
    ARMED.store(false, Ordering::Relaxed);
    if let Ok(mut slot) = STATS_PATH.lock() {
        *slot = Some(stats_path.to_path_buf());
    }
    ALLOCS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
    DEALLOCS.store(0, Ordering::Relaxed);
    DEALLOC_BYTES.store(0, Ordering::Relaxed);
    SAMPLE_SEQ.store(0, Ordering::Relaxed);
    SAMPLED_ALLOCS.store(0, Ordering::Relaxed);
    SAMPLED_BYTES.store(0, Ordering::Relaxed);
    SITE_CAP_DROPS.store(0, Ordering::Relaxed);
    if let Ok(mut sites) = SITES.lock() {
        *sites = if sample_every > 0 {
            Some(HashMap::new())
        } else {
            None
        };
    }
    ARMED.store(true, Ordering::Relaxed);
}

/// The current counters (call before and after the measured window).
#[must_use]
pub fn snapshot() -> Snapshot {
    Snapshot {
        allocs: ALLOCS.load(Ordering::Relaxed),
        alloc_bytes: ALLOC_BYTES.load(Ordering::Relaxed),
        live: LIVE.load(Ordering::Relaxed),
        peak: PEAK.load(Ordering::Relaxed),
    }
}

/// Writes the stats file now. Readers see only atomic tmp+rename
/// publishes; the two potential writers (the 1 s watcher and
/// [`freeze`]) serialize on [`STATS_WRITE_LOCK`] so their tmp
/// write-then-rename critical sections cannot interleave (MINOR-2).
fn write_stats_file() {
    let Some(path) = STATS_PATH.lock().ok().and_then(|slot| slot.clone()) else {
        return;
    };
    if IN_INSTRUMENT.get() {
        return;
    }
    IN_INSTRUMENT.with(|flag| flag.set(true));
    let _guard = STATS_WRITE_LOCK.lock();
    let text = render_stats();
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    } else if !STATS_WRITE_WARNED.swap(true, Ordering::Relaxed) {
        // One-time visibility for the silent-failure case (a typo'd
        // --stats parent dir being the obvious cause); the watcher
        // stays infallible by design (MINOR-5).
        eprintln!(
            "alloc-profile WARNING: stats write to {} failed — no stats file will be \
             produced (further failures suppressed)",
            path.display()
        );
    }
    drop(_guard);
    IN_INSTRUMENT.with(|flag| flag.set(false));
}

fn render_stats() -> String {
    let elapsed = T0.get().map_or(0, |t0| t0.elapsed().as_secs());
    let mut out = String::with_capacity(1 << 16);
    let _ = writeln!(out, "{{");
    let _ = writeln!(out, "\"elapsed_seconds\": {elapsed},");
    // Peak RSS (Linux /proc; 0 where unavailable — a reporting face, not
    // a gate; the brief's `time -v`/`perf stat` alternatives are absent
    // on this machine).
    let _ = writeln!(out, "\"peak_rss_kib\": {},", peak_rss_kib().unwrap_or(0));
    let _ = writeln!(
        out,
        "\"totals\": {{\"allocs\": {}, \"alloc_bytes\": {}, \"deallocs\": {}, \
         \"dealloc_bytes\": {}, \"live\": {}, \"peak_live\": {}}},",
        ALLOCS.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
        DEALLOC_BYTES.load(Ordering::Relaxed),
        LIVE.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
    );
    let _ = writeln!(
        out,
        "\"sampling\": {{\"every\": {}, \"sampled_allocs\": {}, \"sampled_bytes\": {}, \
         \"distinct_cap_drops\": {}}},",
        SAMPLE_EVERY.load(Ordering::Relaxed),
        SAMPLED_ALLOCS.load(Ordering::Relaxed),
        SAMPLED_BYTES.load(Ordering::Relaxed),
        SITE_CAP_DROPS.load(Ordering::Relaxed),
    );
    // Nonzero buckets only: [second, allocs, bytes].
    let _ = writeln!(out, "\"buckets_1s\": [");
    let mut first_bucket = true;
    for (idx, (allocs, bytes)) in BUCKET_ALLOCS
        .iter()
        .zip(BUCKET_BYTES.iter())
        .take((elapsed + 1).min(BUCKETS as u64) as usize)
        .map(|(a, b)| (a.load(Ordering::Relaxed), b.load(Ordering::Relaxed)))
        .enumerate()
    {
        if allocs == 0 && bytes == 0 {
            continue;
        }
        let comma = if first_bucket { "" } else { "," };
        first_bucket = false;
        let _ = writeln!(out, "  {comma}[{idx}, {allocs}, {bytes}]");
    }
    let _ = writeln!(out, "],");
    // Sites ranked by sampled count (bytes as the tiebreak), top N.
    let _ = writeln!(out, "\"sites_ranked_by_sampled_count\": [");
    if let Ok(sites) = SITES.lock()
        && let Some(map) = sites.as_ref()
    {
        {
            let mut rows: Vec<(&String, &(u64, u64))> = map.iter().collect();
            rows.sort_by(|a, b| {
                let (count_a, bytes_a) = *a.1;
                let (count_b, bytes_b) = *b.1;
                count_b
                    .cmp(&count_a)
                    .then_with(|| bytes_b.cmp(&bytes_a))
                    .then_with(|| a.0.cmp(b.0))
            });
            for (rank, (trace, (count, bytes))) in rows.iter().take(TOP_SITES).enumerate() {
                let is_last = rank + 1 == rows.len().min(TOP_SITES);
                let comma = if is_last { "" } else { "," };
                let _ = writeln!(
                    out,
                    "  {{\"count\": {}, \"bytes\": {}, \"trace\": \"{}\"}}{comma}",
                    count,
                    bytes,
                    json_escape(trace)
                );
            }
        }
    }
    let _ = writeln!(out, "]");
    let _ = writeln!(out, "}}");
    out
}

/// Minimal JSON string escaping (the backtrace traces carry quotes and
/// occasionally backslashes; control characters are not produced by the
/// Debug formatter).
fn json_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Peak RSS in KiB from Linux /proc/self/status `VmHWM` (None elsewhere).
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kib);
        }
    }
    None
}
