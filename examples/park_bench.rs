//! Spin `pop`/`push` vs the parking `*_wait` calls vs std `sync_channel`.
//! Throughput (single and batched), ping-pong round trip, idle CPU.
//! `cargo run --release --example park_bench`

extern crate libc;
extern crate spsc;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const CAP: usize = 1024;
const ITEMS: u64 = 10_000_000;
const RUNS: usize = 5;
const PING: usize = 100_000;
const BATCH: usize = 320;

fn check(s: u64) {
    assert_eq!(s, ITEMS * (ITEMS - 1) / 2);
}

fn spin() -> Duration {
    let (p, c) = spsc::make::<u64>(CAP);
    let t = Instant::now();
    let h = thread::spawn(move || (0..ITEMS).fold(0u64, |s, _| s.wrapping_add(c.pop())));
    for i in 0..ITEMS {
        p.push(i);
    }
    check(h.join().unwrap());
    t.elapsed()
}

fn park(spins: u32) -> Duration {
    let (p, c) = spsc::make::<u64>(CAP);
    let t = Instant::now();
    let h =
        thread::spawn(move || (0..ITEMS).fold(0u64, |s, _| s.wrapping_add(c.pop_wait_spin(spins))));
    for i in 0..ITEMS {
        p.push_wait_spin(i, spins);
    }
    check(h.join().unwrap());
    t.elapsed()
}

fn spin_batch() -> Duration {
    let (p, c) = spsc::make::<u64>(CAP);
    let t = Instant::now();
    let h = thread::spawn(move || {
        let mut buf = [0u64; BATCH];
        let (mut got, mut s) = (0u64, 0u64);
        while got < ITEMS {
            let n = c.pop_n(&mut buf);
            for v in &buf[..n] {
                s = s.wrapping_add(*v);
            }
            got += n as u64;
        }
        s
    });
    let mut chunk = [0u64; BATCH];
    let mut i = 0;
    while i < ITEMS {
        for (k, v) in chunk.iter_mut().enumerate() {
            *v = i + k as u64;
        }
        let mut done = 0;
        while done < BATCH {
            done += p.try_push_n(&chunk[done..]);
        }
        i += BATCH as u64;
    }
    check(h.join().unwrap());
    t.elapsed()
}

fn park_batch(spins: u32) -> Duration {
    let (p, c) = spsc::make::<u64>(CAP);
    let t = Instant::now();
    let h = thread::spawn(move || {
        let mut buf = [0u64; BATCH];
        let (mut got, mut s) = (0u64, 0u64);
        while got < ITEMS {
            let n = c.pop_n_wait_spin(&mut buf, spins);
            for v in &buf[..n] {
                s = s.wrapping_add(*v);
            }
            got += n as u64;
        }
        s
    });
    let mut chunk = [0u64; BATCH];
    let mut i = 0;
    while i < ITEMS {
        for (k, v) in chunk.iter_mut().enumerate() {
            *v = i + k as u64;
        }
        p.push_n_wait_spin(&chunk, spins);
        i += BATCH as u64;
    }
    check(h.join().unwrap());
    t.elapsed()
}

fn chan() -> Duration {
    let (p, c) = mpsc::sync_channel::<u64>(CAP);
    let t = Instant::now();
    let h = thread::spawn(move || (0..ITEMS).fold(0u64, |s, _| s.wrapping_add(c.recv().unwrap())));
    for i in 0..ITEMS {
        p.send(i).unwrap();
    }
    check(h.join().unwrap());
    t.elapsed()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn mops<F: Fn() -> Duration>(f: F) -> f64 {
    median(
        (0..RUNS)
            .map(|_| ITEMS as f64 / f().as_secs_f64() / 1e6)
            .collect(),
    )
}

fn pct(mut v: Vec<u64>) -> (u64, u64) {
    v.sort();
    (v[v.len() / 2], v[v.len() * 99 / 100])
}

fn ping_spin() -> (u64, u64) {
    let (atx, arx) = spsc::make::<u64>(CAP);
    let (btx, brx) = spsc::make::<u64>(CAP);
    let h = thread::spawn(move || {
        for _ in 0..PING * 11 / 10 {
            btx.push(arx.pop());
        }
    });
    let mut lat = Vec::new();
    for i in 0..PING * 11 / 10 {
        let t = Instant::now();
        atx.push(i as u64);
        brx.pop();
        if i >= PING / 10 {
            lat.push(t.elapsed().as_nanos() as u64);
        }
    }
    h.join().unwrap();
    pct(lat)
}

fn ping_park(spins: u32) -> (u64, u64) {
    let (atx, arx) = spsc::make::<u64>(CAP);
    let (btx, brx) = spsc::make::<u64>(CAP);
    let h = thread::spawn(move || {
        for _ in 0..PING * 11 / 10 {
            btx.push_wait_spin(arx.pop_wait_spin(spins), spins);
        }
    });
    let mut lat = Vec::new();
    for i in 0..PING * 11 / 10 {
        let t = Instant::now();
        atx.push_wait_spin(i as u64, spins);
        brx.pop_wait_spin(spins);
        if i >= PING / 10 {
            lat.push(t.elapsed().as_nanos() as u64);
        }
    }
    h.join().unwrap();
    pct(lat)
}

fn cpu_us() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

fn idle<F: FnOnce() + Send + 'static, G: FnOnce()>(block: F, release: G) -> f64 {
    let h = thread::spawn(block);
    thread::sleep(Duration::from_millis(50));
    let (c0, w) = (cpu_us(), Instant::now());
    thread::sleep(Duration::from_millis(300));
    let pctv = (cpu_us() - c0) as f64 * 100.0 / w.elapsed().as_micros() as f64;
    release();
    h.join().unwrap();
    pctv
}

fn main() {
    println!("ps-spsc park bench: {ITEMS} u64, capacity {CAP}, median of {RUNS}");
    let base = mops(spin);
    println!("{:<34}{:>10}{:>9}", "impl", "Mitems/s", "vs spin");
    println!("{:<34}{:>10.1}{:>8.0}%", "spin pop/push", base, 100.0);
    for s in [0u32, 64, 256, 1024, 4096] {
        let m = mops(|| park(s));
        println!(
            "{:<34}{:>10.1}{:>8.0}%",
            format!("pop_wait/push_wait spin={s}"),
            m,
            100.0 * m / base
        );
    }
    let bs = mops(spin_batch);
    println!(
        "{:<34}{:>10.1}{:>8.0}%",
        format!("spin pop_n/try_push_n x{BATCH}"),
        bs,
        100.0 * bs / base
    );
    for s in [64u32, 1024, 4096] {
        let m = mops(|| park_batch(s));
        println!(
            "{:<34}{:>10.1}{:>8.0}%",
            format!("pop_n_wait/push_n_wait spin={s}"),
            m,
            100.0 * m / bs
        );
    }
    let m = mops(chan);
    println!(
        "{:<34}{:>10.1}{:>8.0}%",
        "std sync_channel",
        m,
        100.0 * m / base
    );

    println!("\nping-pong round trip, {PING} iters");
    let (a, b) = ping_spin();
    println!("{:<34}p50 {a:>6} ns  p99 {b:>6} ns", "spin");
    for s in [0u32, 1024] {
        let (a, b) = ping_park(s);
        println!(
            "{:<34}p50 {a:>6} ns  p99 {b:>6} ns",
            format!("wait spin={s}")
        );
    }

    println!("\nidle CPU, consumer blocked on an empty queue for 300 ms");
    let (p, c) = spsc::make::<u64>(CAP);
    println!(
        "{:<34}{:>8.3}%",
        "spin pop",
        idle(
            move || {
                c.pop();
            },
            move || p.push(1)
        )
    );
    let (p, c) = spsc::make::<u64>(CAP);
    println!(
        "{:<34}{:>8.3}%",
        "pop_wait",
        idle(
            move || {
                c.pop_wait();
            },
            move || p.push_wait(1)
        )
    );
    let (p, c) = spsc::make::<u64>(1);
    p.push_wait(0);
    println!(
        "{:<34}{:>8.3}%",
        "push_wait on a full queue",
        idle(
            move || p.push_wait(1),
            move || {
                c.pop_wait();
                c.pop_wait();
            }
        )
    );
}
