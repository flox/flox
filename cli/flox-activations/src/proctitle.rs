/// Process title management for setting argv[] visible in ps listings.
///
/// This module implements setproctitle-style functionality by modifying
/// the process's argv memory region.
#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use log::debug;

/// Information about the argv memory region captured at program startup.
struct ArgvMemory {
    start: *mut u8,
    len: usize,
}

unsafe impl Send for ArgvMemory {}
unsafe impl Sync for ArgvMemory {}

static ARGV_MEMORY: OnceLock<Option<ArgvMemory>> = OnceLock::new();

/// Initialize the argv memory tracking.
///
/// This should be called once at program startup before any forks.
/// It locates the argv memory region that can be safely overwritten later.
pub fn init() {
    ARGV_MEMORY.get_or_init(|| {
        unsafe {
            // On Linux, we can use __progname_full or read /proc/self/cmdline
            // to find where argv lives. We'll calculate the available space
            // from argv through the end of environ.

            unsafe extern "C" {
                static mut environ: *mut *mut libc::c_char;
            }

            if environ.is_null() {
                debug!("proctitle::init: environ is null");
                return None;
            }

            // Find the start of argv by reading /proc/self/cmdline
            let cmdline = match std::fs::read("/proc/self/cmdline") {
                Ok(data) => data,
                Err(e) => {
                    debug!("proctitle::init: failed to read cmdline: {}", e);
                    return None;
                },
            };

            if cmdline.is_empty() {
                return None;
            }

            // Find the last environment variable to calculate the end
            let mut env_ptr = environ;
            let mut argv_start: *const u8 = std::ptr::null();
            let mut argv_end: *const u8 = std::ptr::null();

            // Find minimum and maximum addresses in environ
            let mut count = 0;
            while !(*env_ptr).is_null() && count < 10000 {
                let env_str = *env_ptr as *const u8;
                let len = libc::strlen(*env_ptr);
                let env_end = env_str.add(len + 1);

                if argv_start.is_null() || env_str < argv_start {
                    argv_start = env_str;
                }
                if env_end > argv_end {
                    argv_end = env_end;
                }

                env_ptr = env_ptr.add(1);
                count += 1;
            }

            if argv_start.is_null() || argv_end.is_null() {
                debug!("proctitle::init: could not determine argv bounds");
                return None;
            }

            // Now search backwards from environ for our argv
            // The cmdline tells us what argv contains, so we search for it
            let search_size = 8192; // Search up to 8KB before environ
            let search_start = (argv_start as usize).saturating_sub(search_size);

            // Try to find our first argument in this region
            let args: Vec<String> = std::env::args().collect();
            if args.is_empty() {
                return None;
            }

            let first_arg_bytes = args[0].as_bytes();

            // Search for the first argument in memory
            for offset in 0..search_size {
                let check_addr = (search_start + offset) as *const u8;
                let mut matches = true;

                // Check if this location matches our first argument
                for (i, &byte) in first_arg_bytes.iter().enumerate() {
                    if *check_addr.add(i) != byte {
                        matches = false;
                        break;
                    }
                }

                if matches && *check_addr.add(first_arg_bytes.len()) == 0 {
                    // Found it! This is our argv[0]
                    let total_len = argv_end as usize - check_addr as usize;

                    if total_len > 0 && total_len < 1024 * 1024 {
                        debug!(
                            "proctitle::init: found argv at {:p}, {} bytes available",
                            check_addr, total_len
                        );

                        return Some(ArgvMemory {
                            start: check_addr as *mut u8,
                            len: total_len,
                        });
                    }
                }
            }

            debug!("proctitle::init: could not locate argv[0] in memory");
            None
        }
    });
}

/// Set the process title that appears in ps listings.
///
/// This function modifies both the process "comm" name (via prctl) and
/// the full argv memory region, making the title visible in ps output.
///
/// # Arguments
///
/// * `title` - The new process title to display
pub fn setproctitle(title: &str) -> Result<()> {
    // Method 1: Set via prctl for the comm field (limited to 15 chars)
    #[cfg(target_os = "linux")]
    set_comm_name(title)?;

    // Method 2: Overwrite the argv memory region if we have it
    if let Some(Some(argv_mem)) = ARGV_MEMORY.get() {
        set_argv_memory(title, argv_mem)?;
    } else {
        debug!("proctitle: argv memory not initialized, using comm name only");
    }

    Ok(())
}

/// Set the process comm name via prctl (limited to 15 characters).
#[cfg(target_os = "linux")]
fn set_comm_name(title: &str) -> Result<()> {
    let comm_title = if title.len() > 15 {
        &title[..15]
    } else {
        title
    };

    unsafe {
        let c_title = CString::new(comm_title).context("Failed to create CString for prctl")?;

        let result = libc::prctl(
            libc::PR_SET_NAME,
            c_title.as_ptr() as libc::c_ulong,
            0,
            0,
            0,
        );

        if result == 0 {
            debug!("proctitle: set comm name to: {}", comm_title);
        } else {
            debug!("proctitle: prctl PR_SET_NAME failed");
        }
    }

    Ok(())
}

/// Overwrite the argv memory region with the new title.
fn set_argv_memory(title: &str, argv_mem: &ArgvMemory) -> Result<()> {
    unsafe {
        let title_bytes = title.as_bytes();
        let copy_len = std::cmp::min(title_bytes.len(), argv_mem.len - 1);

        // Zero out the entire region
        libc::memset(argv_mem.start as *mut libc::c_void, 0, argv_mem.len);

        // Copy the new title
        libc::memcpy(
            argv_mem.start as *mut libc::c_void,
            title_bytes.as_ptr() as *const libc::c_void,
            copy_len,
        );

        debug!(
            "proctitle: set full process title ({} bytes): {}",
            copy_len, title
        );
    }

    Ok(())
}
