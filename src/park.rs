//! Wait on a 32-bit word and wake its waiter: futex on Linux and Android,
//! `os_sync_wait_on_address` on Apple (macOS 14.4+, looked up at runtime) with
//! `__ulock_wait` as the fallback. No inline asm and no std.
//!
//! `wait` may return spuriously; callers re-check their condition in a loop.
//! On any other target `wait` is a spin hint and `wake` does nothing, so the
//! blocking calls degrade to the old spin-wait instead of failing to build.

use core::sync::atomic::AtomicU32;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use super::AtomicU32;

    #[inline]
    pub(crate) fn wait(word: &AtomicU32, expected: u32) {
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word as *const AtomicU32,
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                expected,
                core::ptr::null::<libc::timespec>(),
            );
        }
    }

    #[inline]
    pub(crate) fn wake(word: &AtomicU32) {
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                word as *const AtomicU32,
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                1i32,
            );
        }
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::AtomicU32;
    use core::ffi::{c_int, c_void};
    use core::sync::atomic::{AtomicUsize, Ordering};

    const UL_COMPARE_AND_WAIT: u32 = 1;
    const ULF_NO_ERRNO: u32 = 0x0100_0000;

    extern "C" {
        fn __ulock_wait(operation: u32, addr: *mut c_void, value: u64, timeout_us: u32) -> c_int;
        fn __ulock_wake(operation: u32, addr: *mut c_void, wake_value: u64) -> c_int;
    }

    type OsWait = unsafe extern "C" fn(*mut c_void, u64, usize, u32) -> c_int;
    type OsWake = unsafe extern "C" fn(*mut c_void, usize, u32) -> c_int;

    // 0 = not looked up yet, 1 = not available, anything else = the function.
    static OS_WAIT: AtomicUsize = AtomicUsize::new(0);
    static OS_WAKE: AtomicUsize = AtomicUsize::new(0);

    #[cold]
    fn resolve() {
        let wait = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                b"os_sync_wait_on_address\0".as_ptr() as *const libc::c_char,
            )
        };
        let wake = unsafe {
            libc::dlsym(
                libc::RTLD_DEFAULT,
                b"os_sync_wake_by_address_any\0".as_ptr() as *const libc::c_char,
            )
        };
        let (wait, wake) = if wait.is_null() || wake.is_null() {
            (1, 1)
        } else {
            (wait as usize, wake as usize)
        };
        OS_WAKE.store(wake, Ordering::Relaxed);
        OS_WAIT.store(wait, Ordering::Release);
    }

    #[inline]
    fn api() -> Option<(OsWait, OsWake)> {
        let mut wait = OS_WAIT.load(Ordering::Acquire);
        if wait == 0 {
            resolve();
            wait = OS_WAIT.load(Ordering::Acquire);
        }
        if wait == 1 {
            return None;
        }
        let wake = OS_WAKE.load(Ordering::Relaxed);
        unsafe {
            Some((
                core::mem::transmute::<usize, OsWait>(wait),
                core::mem::transmute::<usize, OsWake>(wake),
            ))
        }
    }

    #[inline]
    pub(crate) fn wait(word: &AtomicU32, expected: u32) {
        let addr = word as *const AtomicU32 as *mut c_void;
        unsafe {
            match api() {
                Some((wait, _)) => {
                    wait(addr, u64::from(expected), 4, 0);
                }
                None => {
                    __ulock_wait(
                        UL_COMPARE_AND_WAIT | ULF_NO_ERRNO,
                        addr,
                        u64::from(expected),
                        0,
                    );
                }
            }
        }
    }

    #[inline]
    pub(crate) fn wake(word: &AtomicU32) {
        let addr = word as *const AtomicU32 as *mut c_void;
        unsafe {
            match api() {
                Some((_, wake)) => {
                    wake(addr, 4, 0);
                }
                None => {
                    __ulock_wake(UL_COMPARE_AND_WAIT | ULF_NO_ERRNO, addr, 0);
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
mod imp {
    use super::AtomicU32;

    #[inline]
    pub(crate) fn wait(_word: &AtomicU32, _expected: u32) {
        core::hint::spin_loop();
    }

    #[inline]
    pub(crate) fn wake(_word: &AtomicU32) {}
}

pub(crate) use self::imp::{wait, wake};
