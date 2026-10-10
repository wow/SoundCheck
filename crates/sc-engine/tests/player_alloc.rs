//! The audio callback never allocates: it runs here under a global allocator that counts every
//! allocation made while counting is switched on, through playing, switching versions (the gain
//! ramp), level matching, volume changes down to mute, a seek and a pause. This binary holds this
//! one test, and only allocations on its thread are counted.
#![allow(unsafe_code)] // a global allocator is an unsafe trait; each method only forwards

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use sc_core::DbFs;
use sc_core::ipc::{Listen, Version};
use sc_engine::player::{Callback, Feeder, Meters, Renderer, Shared, ring};
use sc_engine::{Track, TrackProgress};

struct Counting;

thread_local! {
    /// Set only on the thread under test: the harness's own thread formats and prints the test's
    /// progress meanwhile, and its allocations are not the code's.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator with the caller's arguments unchanged,
// so `Counting` upholds exactly the contract `System` does; the count is an atomic and the
// switch a const thread-local without a destructor, neither of which allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: the caller's layout, passed on as `GlobalAlloc::alloc` requires.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator, which is `System`, with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as for `dealloc`; `new_size` is the caller's, checked by the caller.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn the_callback_never_allocates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..88_200_i32 {
        let v = i16::try_from((n * 37) % 20_000).unwrap();
        w.write_sample(v).unwrap();
        w.write_sample(v).unwrap();
    }
    w.finalize().unwrap();
    let (tx, rx) = mpsc::channel();
    let track = Arc::new(
        Track::open(&path, DbFs(-1.0), move |p| {
            if !matches!(p, TrackProgress::Decoded(_)) {
                let _ = tx.send(());
            }
        })
        .unwrap(),
    );
    rx.recv().unwrap();

    let shared = Arc::new(Shared::default());
    let (producer, consumer) = ring(48_000, 2);
    let renderer = Renderer::new(track, 48_000, 2).unwrap();
    let meters = Arc::new(Meters::new());
    let mut feeder =
        Feeder::new(renderer, producer, Arc::clone(&shared), meters, 48_000, 2).unwrap();
    feeder.set_planned_gain(DbFs(-4.5));
    let mut callback = Callback::new(consumer, Arc::clone(&shared), 2, 48_000);
    let mut buffer = vec![0.0_f32; 512];
    feeder.play();

    // The count sees this thread's allocations, so a zero below means none were made.
    COUNTING.set(true);
    drop(std::hint::black_box(Vec::<u8>::with_capacity(1)));
    COUNTING.set(false);
    assert_eq!(
        ALLOCATIONS.swap(0, Ordering::SeqCst),
        1,
        "one allocation counted"
    );
    let mut measured = |feeder: &mut Feeder, calls: usize| {
        for _ in 0..calls {
            feeder.pump();
            COUNTING.set(true);
            callback.fill(&mut buffer);
            COUNTING.set(false);
        }
    };
    measured(&mut feeder, 50);
    // Each change lands mid-ramp of the one before (10 ms is under two 256-frame buffers).
    for listen in [
        (Version::Original, false),
        (Version::Processed, false),
        (Version::Processed, true),
        (Version::Original, true),
    ] {
        feeder.set_listen(Listen {
            version: listen.0,
            matched: listen.1,
        });
        measured(&mut feeder, 1);
    }
    for volume in [Some(DbFs(-12.0)), None, Some(DbFs(0.0))] {
        feeder.set_volume(volume);
        measured(&mut feeder, 3);
    }
    feeder.seek(30_000);
    measured(&mut feeder, 50);
    feeder.set_listen(Listen::default());
    feeder.pause();
    measured(&mut feeder, 10);
    assert_eq!(ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert!(shared.played.load(Ordering::SeqCst) > 0);
}
