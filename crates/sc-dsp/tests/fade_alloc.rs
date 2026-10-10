//! Fading allocates nothing per block: [`FadeIn::push`] and a [`Requantiser`] with a fade-in
//! run here under a global allocator that counts every allocation made while counting is
//! switched on, over blocks of 1, 7 and 4096 frames that start inside and after the ramp. This
//! binary holds this one test, so no other test allocates while the count runs.
#![allow(unsafe_code)] // a global allocator is an unsafe trait; each method only forwards

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sc_dsp::{FadeIn, Requantiser, SourceDepth};

struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator with the caller's arguments unchanged,
// so `Counting` upholds exactly the contract `System` does; the counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
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
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: as for `dealloc`; `new_size` is the caller's, checked by the caller.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn fading_never_allocates_per_block() {
    let input: Vec<i32> = (0..20_000).map(|i| (i * 37) % 30_000 - 15_000).collect();
    let floats: Vec<f64> = input.iter().map(|x| f64::from(*x) / 32_768.0).collect();
    let mut out = vec![0_i32; input.len()];
    let mut scratch = floats.clone();
    let mut fade = FadeIn::new(96, 2).expect("valid");
    let mut quant = Requantiser::new(SourceDepth::Int { bits: 16 }, 16, -3.2, 5).expect("valid");
    quant.set_fade_in(FadeIn::new(88, 2).expect("valid"));
    let mut quant_f = Requantiser::new(SourceDepth::Float, 24, 0.0, 5).expect("valid");
    quant_f.set_fade_in(FadeIn::new(88, 2).expect("valid"));
    COUNTING.store(true, Ordering::Relaxed);
    let mut at = 0;
    for size in [2, 14, 8192].iter().cycle().take(6) {
        let end = (at + size).min(input.len());
        fade.push(&mut scratch[at..end]);
        quant
            .push_int(&input[at..end], &mut out[at..end])
            .expect("integer source");
        quant_f
            .push_float(&floats[at..end], &mut out[at..end])
            .expect("float source");
        at = end;
    }
    COUNTING.store(false, Ordering::Relaxed);
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), 0);
    assert!(fade.is_done() && at > 192);
}
