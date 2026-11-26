//! Real-time scheduling for the transport thread.
//!
//! On macOS the thread asks Mach for a time-constraint policy (the class Core
//! Audio uses): the scheduler then wakes it within tens of microseconds instead
//! of the ~1 ms overshoot an ordinary thread sees.

#[cfg(target_os = "macos")]
pub fn promote_current_thread() -> bool {
    use mach2::kern_return::KERN_SUCCESS;
    use mach2::mach_time::{mach_timebase_info, mach_timebase_info_data_t};
    use mach2::thread_policy::{
        THREAD_TIME_CONSTRAINT_POLICY, THREAD_TIME_CONSTRAINT_POLICY_COUNT, thread_policy_set,
        thread_time_constraint_policy_data_t,
    };

    unsafe extern "C" {
        fn pthread_mach_thread_np(thread: libc_pthread_t) -> u32;
        fn pthread_self() -> libc_pthread_t;
    }
    #[allow(non_camel_case_types)]
    type libc_pthread_t = usize;

    unsafe {
        let mut timebase = mach_timebase_info_data_t { numer: 0, denom: 0 };
        mach_timebase_info(&mut timebase);
        let ns_to_abs = |ns: f64| (ns * f64::from(timebase.denom) / f64::from(timebase.numer)) as u32;
        // Every ~1 ms we may need ~0.2 ms of CPU, finished within 0.5 ms.
        let mut policy = thread_time_constraint_policy_data_t {
            period: ns_to_abs(1_000_000.0),
            computation: ns_to_abs(200_000.0),
            constraint: ns_to_abs(500_000.0),
            preemptible: 1,
        };
        let thread = pthread_mach_thread_np(pthread_self());
        let result = thread_policy_set(
            thread,
            THREAD_TIME_CONSTRAINT_POLICY,
            (&raw mut policy).cast(),
            THREAD_TIME_CONSTRAINT_POLICY_COUNT,
        );
        result == KERN_SUCCESS
    }
}

#[cfg(not(target_os = "macos"))]
pub fn promote_current_thread() -> bool {
    false
}

/// Block until monotonic time `t` (seconds, see [`crate::clock::monotonic`]).
///
/// On macOS this is `mach_wait_until`, which on a time-constraint thread wakes
/// within tens of microseconds; condition-variable timeouts are coalesced by
/// the kernel and overshoot by 1-4 ms even on real-time threads (measured).
#[cfg(target_os = "macos")]
pub fn sleep_until(t: f64) {
    use mach2::mach_time::{mach_absolute_time, mach_timebase_info, mach_timebase_info_data_t, mach_wait_until};
    let remaining = t - crate::clock::monotonic();
    if remaining <= 0.0 {
        return;
    }
    unsafe {
        let mut tb = mach_timebase_info_data_t { numer: 0, denom: 0 };
        mach_timebase_info(&mut tb);
        let ticks = remaining * 1e9 * f64::from(tb.denom) / f64::from(tb.numer);
        mach_wait_until(mach_absolute_time() + ticks as u64);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn sleep_until(t: f64) {
    let remaining = t - crate::clock::monotonic();
    if remaining > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(remaining));
    }
}
