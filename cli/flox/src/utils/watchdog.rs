//
// The Flox watchdog function simply forks and immediately returns controller
// to the caller in the parent process, while the child process waits for the
// parent process to die. This is useful for performing cleanup tasks and other
// processing parent process is concluded.
//
// At present this function is called to monitor the lifetime of `flox activate`
// invocations for the purpose of removing temporary files required during their
// lifetime, but may also be useful for enabling "straight-line" execution of
// other flox subcommands, e.g. for the purpose of performing metrics submission
// in the background, potentially until well after the main process has finished.
//
// See https://github.com/flox/flox/issues/1500 for more information.
//
use std::io::Result;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

#[cfg(target_os = "linux")]
use libc::{prctl, PR_SET_PDEATHSIG};
use log::debug;
#[cfg(target_os = "linux")]
use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, SIGUSR1};
use nix::unistd::{fork, getpid, getppid, ForkResult, Pid};
use once_cell::sync::OnceCell;
use proctitle::set_title;

// Use global variable to make it possible to access variables from
// signal handler on Linux.
static START_TIME: OnceCell<SystemTime> = OnceCell::new();
static END_TIME: OnceCell<SystemTime> = OnceCell::new();

// Linux implementation adapted from:
// https://github.com/iximiuz/reapme/blob/master/src/sleepy.rs
#[cfg(target_os = "linux")]
extern "C" fn handle_sigusr1(_: libc::c_int) {
    let _ = END_TIME.set(SystemTime::now());
}

#[cfg(target_os = "linux")]
fn wait_parent_pid(pid: Pid) -> Result<()> {
    // Linux uses PR_SET_PDEATHSIG to communicate parent death to child.
    unsafe {
        prctl(PR_SET_PDEATHSIG, SIGUSR1);
    }
    let sig_action = SigAction::new(
        SigHandler::Handler(handle_sigusr1),
        SaFlags::empty(),
        SigSet::empty(),
    );
    if let Err(err) = unsafe { sigaction(SIGUSR1, &sig_action) } {
        println!("[watchdog] sigaction() failed: {}", err);
    };
    Ok(())
}

// MacOS implementation ported from mac-pid-waiter.c found in:
// https://unix.stackexchange.com/questions/427255/child-process-listen-for-event-when-parent-dies
#[cfg(target_os = "macos")]
fn wait_parent_pid(pid: Pid) -> Result<()> {
    let mut watcher = kqueue::Watcher::new()?;
    watcher.add_pid(
        pid.into(),
        kqueue::EventFilter::EVFILT_PROC,
        kqueue::FilterFlag::NOTE_EXIT,
    )?;
    watcher.watch()?;
    // The only event coming our way is the exit event for
    // the parent pid, so just grab it and continue.
    let _ = watcher.iter().next();
    let _ = END_TIME.set(SystemTime::now());
    Ok(())
}

// Watchdog subroutine that forks, then the returns control to the caller
// from the parent while the child awaits the death of its parent.
pub fn in_watchdog_process() -> bool {
    START_TIME
        .set(SystemTime::now())
        .expect("START_TIME can only be set once");

    // Gather all config prior to forking (makes it easier to debug).
    let flox_pid = getpid();
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => {
            debug!("forked watchdog with pid {}", child);
            return false;
        },
        Ok(ForkResult::Child) => {
            // continue below
        },
        Err(err) => panic!("main: fork failed: {}", err),
    };

    // Set the process title to "flox-watchdog". This is useful but
    // only works on Linux.
    set_title("flox activate watchdog for PID {flox_pid}");

    // Assert we have in fact been forked.
    assert_eq!(flox_pid, getppid());

    // wait for parent pid to die
    // TODO: factor this out better.
    debug!(
        "[watchdog] pid is {}, waiting for pid {} to die",
        getpid(),
        flox_pid
    );

    if let Err(err) = wait_parent_pid(flox_pid) {
        println!("{:?}", err);
    }

    // Loop waiting for END_TIME to be set:
    // - macos: it will be already be set by wait_parent_pid
    // - linux: we're waiting for SIGUSR1 to set it for us
    // It's fine to loop because it is not resource-intensive
    // and this is an async metrics submission process anyway.
    while END_TIME.get().is_none() {
        sleep(Duration::from_millis(1000)); // Sleep to prevent busy waiting
    }

    // Compute and print the elapsed duration
    if let (Some(start_time), Some(end_time)) = (START_TIME.get(), END_TIME.get()) {
        match end_time.duration_since(*start_time) {
            Ok(elapsed) => {
                debug!("[watchdog] elapsed time: {:?}", elapsed);
            },
            Err(e) => {
                eprintln!("Error calculating elapsed time: {:?}", e);
            },
        }
    }
    // FIXME: This is a temporary workaround for the fact that the rust
    // child process throws a backtrace when it exits:
    //
    //     $ flox activate -d /tmp/asdlkfjasdlkfj -- sleep 3
    //     thread 'main' panicked at /Users/brantley/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.37.0/src/runtime/io/driver.rs:209:27:
    //     failed to wake I/O driver: Os { code: 9, kind: Uncategorized, message: "Bad file descriptor" }
    //     note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
    //
    // My guess is that the child process is trying to process state
    // established by its parent, and either the Sentry code or our code
    // isn't expecting that. The fix for that may be to delay such
    // initialization until after the fork, but I'd want a Rust expert
    // to confirm that. For now we just exit(0) which is roughly equivalent
    // to the behavior we had before.
    // true
    std::process::exit(0)
}
