
## Bounded SPSC Queue

A PathScale fork of [polyfractal/bounded-spsc-queue](https://github.com/polyfractal/bounded-spsc-queue), published as `ps-spsc`. It builds without the standard library, pads to the 128-byte line on Apple silicon, and adds a bulk `pop_n`.

This crate provides a very simple bounded, Single-producer Single-consumer (SPSC)
queue for Rust.  It provides a data structure for two threads to communicate
in one direction with minimal overhead and bounded semantics.

Compared to a `sync_channel`, this queue provides a small but consistent
speedup.  `sync_channel` utilizes an unbounded linked-list data structure under the covers,
while `spsc` is a simple ring buffer with single, solid block of allocated
memory.  The solid block of memory allows better cache pre-fetching due to less pointer
indirection, and generally simpler operations to achieve a bounded SPSC queue.

`make` rounds the requested capacity up to a power of two using checked
arithmetic. It panics if rounding or allocation-size computation overflows, or
if the allocation layout exceeds `isize::MAX` bytes. Capacity zero is supported
and accepts no values. Zero-sized element types need no ring storage allocation.
Allocation failure is handled by the allocator's allocation error handler.

`pop_n` replaces existing target values. For types with destructors, each
transfer is published before the old target value is dropped. If a destructor
panics, completed transfers remain consumed and all target slots remain
initialized. These transfers also wake a parked producer. Types without
destructors retain the bulk-copy path.

The blocking `*_wait` calls use paired light and heavy barriers when checking
parked peers. Pair producer and consumer wait calls when either side may park;
spinning `pop` and `try_pop` do not wake a parked producer.

On Apple platforms, barrier scans skip terminated threads and dead thread
destinations. Other scan failures use a local fence without changing the
selected barrier mode. Apple waits request a 1 ms timeout and recheck the queue,
so a missed wakeup does not cause an indefinite sleep. Idle Apple waiters
therefore wake periodically. Linux and Android waits retain their existing
behavior.

## Documentation

[Documentation can be found here](https://docs.rs/ps-spsc)

## Example

```rust
use std::thread;
use spsc::{Producer, Consumer};

// Initialize a queue with capacity of 500 values
let (p, c) = spsc::make(500);

// Spawn a new thread and move the Producer into it
thread::spawn(move|| {
  for i in 0..100000 {
    p.push(i);
  }
});

// Back in the first thread, start pop'ing values off the queue
for i in 0..100000 {
  let t = c.pop();
  assert!(t == i);
}
```

## Semi-scientific Benchmarks
On my Macbook Air (dual-core 1.7 GHz Intel Core i7), `sync_channel` can perform
single threaded `send()/recv()` paired operations in ~200ns.  

![Sync Chan PDF](/sync_chan_pdf.png)

`spsc` performs the same test in ~10ns.
![SPSC PDF](/spsc_pdf.png)

A more realistic, although harder test to accurately benchmark, is threaded
performance.  This test spins up a "listener" thread which attempts to drain
the queue as fast as possible.  It must intermittently check an Atomic flag
to determine if the test is over, which unfortunately will impact the benchmark results.
To help alleviate the impact, the listener will only check every 500 iterations.

In the original thread, the bencher attempts to push data onto the queue.
This is where timing can get flaky: if the listener is not draining fast enough,
the producer may stall while waiting for the queue to free a spot.  This test
should not be viewed as a latency test, but more as a throughput test since
it is really testing both producer and consumer performance in conjunction.

`sync_channel` scores ~6μs on this test:

![Sync Chan Threaded PDF](/sync_chan_threaded_pdf.png)

While `spsc` scores ~20ns:

![SPSC Threaded PDF](/spsc_threaded_pdf.png)

A second set of benchmarks look at the inverse (secondary thread pushes onto the
queue continuously, while the main thread pops data off).  The timings are very
similar.

Put another way:
- SPSC performs ~50m push operations per second
- Sync Channel performs ~170k operations per second
