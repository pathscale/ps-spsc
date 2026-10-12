//! Asymmetric fences for the parking handoff. `light` sits on the hot side
//! (a publish followed by a check of the peer parked flag) and `heavy` on the
//! side that is about to park. When the heavy side can force a barrier on every
//! thread of the process, the light side needs only a compiler fence; otherwise
//! both sides fall back to a SeqCst fence. No inline asm and no std.
//!
//! Linux and Android: `membarrier(MEMBARRIER_CMD_PRIVATE_EXPEDITED)`, after a
//! successful registration. Apple: `thread_get_register_pointer_values` on every
//! thread of the task, which interrupts each running thread to read its
//! registers. That is what .NET uses for `FlushProcessWriteBuffers` on macOS;
//! Apple does not document it as a barrier.

use core::sync::atomic::{compiler_fence, fence, AtomicU8, Ordering};

const UNKNOWN: u8 = 0;
const ASYMMETRIC: u8 = 1;
const FALLBACK: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Decide once whether the heavy barrier works. Called by `make`, before any
/// queue exists, so every `light` on a queue sees the final state.
pub(crate) fn init() {
    if STATE.load(Ordering::Relaxed) == UNKNOWN {
        let s = if imp::register() {
            ASYMMETRIC
        } else {
            FALLBACK
        };
        STATE.store(s, Ordering::Relaxed);
    }
}

#[inline(always)]
pub(crate) fn light() {
    if STATE.load(Ordering::Relaxed) == ASYMMETRIC {
        compiler_fence(Ordering::SeqCst);
    } else {
        fence(Ordering::SeqCst);
    }
}

#[inline]
pub(crate) fn heavy() {
    fence(Ordering::SeqCst);
    if STATE.load(Ordering::Relaxed) == ASYMMETRIC {
        imp::barrier();
        fence(Ordering::SeqCst);
    }
}

#[cfg(all(not(miri), any(target_os = "linux", target_os = "android")))]
mod imp {
    const CMD_PRIVATE_EXPEDITED: libc::c_long = 1 << 3;
    const CMD_REGISTER_PRIVATE_EXPEDITED: libc::c_long = 1 << 4;

    pub(super) fn register() -> bool {
        unsafe {
            libc::syscall(
                libc::SYS_membarrier,
                CMD_REGISTER_PRIVATE_EXPEDITED,
                0 as libc::c_long,
            ) == 0
                && libc::syscall(
                    libc::SYS_membarrier,
                    CMD_PRIVATE_EXPEDITED,
                    0 as libc::c_long,
                ) == 0
        }
    }

    pub(super) fn barrier() {
        let r = unsafe {
            libc::syscall(
                libc::SYS_membarrier,
                CMD_PRIVATE_EXPEDITED,
                0 as libc::c_long,
            )
        };
        // Registration succeeded, so a failure here means the kernel broke its
        // contract and the light side is no longer ordered.
        if r != 0 {
            panic!("membarrier failed after registration");
        }
    }
}

#[cfg(all(not(miri), target_vendor = "apple"))]
mod imp {
    type Port = u32;
    const KERN_SUCCESS: i32 = 0;
    const KERN_INSUFFICIENT_BUFFER_SIZE: i32 = 52;

    extern "C" {
        static mach_task_self_: Port;
        fn task_threads(task: Port, list: *mut *mut Port, count: *mut u32) -> i32;
        fn thread_get_register_pointer_values(
            thread: Port,
            sp: *mut usize,
            length: *mut usize,
            values: *mut usize,
        ) -> i32;
        fn mach_port_deallocate(task: Port, name: Port) -> i32;
        fn vm_deallocate(task: Port, address: usize, size: usize) -> i32;
    }

    /// Returns false if any live thread could not be interrupted.
    fn flush() -> bool {
        unsafe {
            let task = mach_task_self_;
            let mut list: *mut Port = core::ptr::null_mut();
            let mut count = 0u32;
            if task_threads(task, &mut list, &mut count) != KERN_SUCCESS {
                return false;
            }
            let mut ok = true;
            let mut regs = [0usize; 128];
            for i in 0..count as usize {
                let th = *list.add(i);
                let mut sp = 0usize;
                let mut len = regs.len();
                let r =
                    thread_get_register_pointer_values(th, &mut sp, &mut len, regs.as_mut_ptr());
                if r != KERN_SUCCESS && r != KERN_INSUFFICIENT_BUFFER_SIZE {
                    ok = false;
                }
                mach_port_deallocate(task, th);
            }
            vm_deallocate(
                task,
                list as usize,
                count as usize * core::mem::size_of::<Port>(),
            );
            ok
        }
    }

    pub(super) fn register() -> bool {
        flush()
    }

    pub(super) fn barrier() {
        if !flush() {
            panic!("thread_get_register_pointer_values failed after a successful probe");
        }
    }
}

#[cfg(any(
    miri,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
mod imp {
    pub(super) fn register() -> bool {
        false
    }

    pub(super) fn barrier() {}
}
