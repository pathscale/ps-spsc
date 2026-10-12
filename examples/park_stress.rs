//! Lost-wakeup stress for the parking calls. Tiny capacities and a zero spin
//! budget force both sides through the park path; any lost wakeup hangs, and
//! a watchdog turns that into a failure. `cargo run --release --example park_stress`

extern crate spsc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn stream(cap: usize, items: u64, spins: u32) {
    let (p, c) = spsc::make::<u64>(cap);
    let h = thread::spawn(move || {
        let mut s = 0u64;
        for i in 0..items {
            let v = c.pop_wait_spin(spins);
            assert_eq!(v, i, "out of order");
            s = s.wrapping_add(v);
        }
        s
    });
    for i in 0..items {
        p.push_wait_spin(i, spins);
    }
    assert_eq!(h.join().unwrap(), items * (items - 1) / 2);
}

fn batched(cap: usize, items: u64, spins: u32) {
    let (p, c) = spsc::make::<u64>(cap);
    let h = thread::spawn(move || {
        let mut buf = [0u64; 7];
        let mut next = 0u64;
        while next < items {
            let n = c.pop_n_wait_spin(&mut buf, spins);
            for v in &buf[..n] {
                assert_eq!(*v, next);
                next += 1;
            }
        }
    });
    let mut i = 0;
    while i < items {
        let k = (items - i).min(5);
        let chunk: Vec<u64> = (i..i + k).collect();
        p.push_n_wait_spin(&chunk, spins);
        i += k;
    }
    h.join().unwrap();
}

fn main() {
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    thread::spawn(move || {
        let t = Instant::now();
        while !d.load(Ordering::Relaxed) {
            if t.elapsed() > Duration::from_secs(600) {
                eprintln!("watchdog: no progress, a wakeup was lost");
                std::process::exit(1);
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let t = Instant::now();
    stream(1, 10_000_000, 0);
    println!(
        "cap 1, spin 0: 10000000 handoffs ok ({:.1}s)",
        t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    stream(4, 10_000_000, 16);
    println!(
        "cap 4, spin 16: 10000000 handoffs ok ({:.1}s)",
        t.elapsed().as_secs_f64()
    );
    let t = Instant::now();
    batched(8, 10_000_000, 0);
    println!(
        "cap 8 batched, spin 0: 10000000 items ok ({:.1}s)",
        t.elapsed().as_secs_f64()
    );
    done.store(true, Ordering::Relaxed);
    println!("no lost wakeups");
}
