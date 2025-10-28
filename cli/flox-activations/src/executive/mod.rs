use libc::kill;
use log::debug;

pub fn executive() {
    debug!("executive called");

    // Send SIGUSR1 to parent process to let them know that the process is ready
    // to be attached to.
    unsafe {
        libc::kill(libc::getppid(), libc::SIGUSR1);
    }

    // sleep indefinitely to simulate running executive
    // In a real scenario, this would be the main loop of the executive process
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
