use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(target_os = "linux")]
use nix::libc::{PR_SET_CHILD_SUBREAPER, prctl};
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork, getpid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;

#[derive(Clone, Debug)]
struct ActivateData {
    env: String,
    shell: String,
    mode: String,
}

fn reaper() {
    let pid = getpid();
    nix::unistd::setsid().expect("setsid failed");
    println!("[executive {}] created new session with setsid()", pid);

    #[cfg(target_os = "linux")]
    unsafe {
        prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
        println!("[executive {}] registered as subreaper", pid);
    }

    let mut signals = Signals::new([SIGCHLD]).expect("failed to create Signals");
    println!(
        "[executive {}] created SIGCHLD handler (spawned a thread!)",
        pid
    );

    // Probably doesn't matter about shutting this thread or the signal handler
    // down since it will exit with the executive process
    std::thread::spawn(move || {
        for signal in signals.forever() {
            if signal == SIGCHLD {
                // Reap all available children
                loop {
                    match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::StillAlive) => break,
                        Ok(status) => {
                            println!("[executive reaper {}] reaped child: {:?}", getpid(), status)
                        },
                        Err(_) => break,
                    }
                }
            }
        }
    });
}

fn activate_script(data: &ActivateData) {
    let pid = getpid();
    // Build the activate script command
    let mut command = Command::new("/bin/echo");
    command.arg("[executive script] ran activate script");
    command.arg(&data.env);
    command.arg(&data.shell);
    command.arg(&data.mode);
    command
        .stderr(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit());

    println!("[executive {}] running activation script", pid);

    match command.status() {
        Ok(status) => println!("[executive {}] activation child exited: {:?}", pid, status),
        Err(e) => eprintln!("[executive {}] failed to run: {}", pid, e),
    }
}

fn process_compose_background() -> std::process::Child {
    let pid = getpid();
    println!("[executive {}] starting process-compose (sleep)", pid);

    Command::new("/bin/sleep")
        .arg("10m")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn process-compose")
}

fn executive(data: ActivateData, parent_pid: Pid) {
    let pid = getpid();

    // Linux only, and even then not sure it works.
    let argv: Vec<String> = std::env::args().collect();
    let new_title = format!("flox-executive: {}", argv.join(" "));
    proctitle::set_title(&new_title);

    println!("[executive {}] started, parent_pid: {}", pid, parent_pid);

    reaper(); // Technically this should all be Linux only but including to test threads
    activate_script(&data);

    let mut pc_child = process_compose_background();
    println!(
        "[executive {}] process-compose running (pid {})",
        pid,
        pc_child.id()
    );

    println!(
        "[executive {}] sending SIGUSR1 to parent (pid {})",
        pid, parent_pid
    );
    kill(parent_pid, Signal::SIGUSR1).expect("failed to send SIGUSR1");

    println!(
        "[executive {}] monitoring parent (pid {})...",
        pid, parent_pid
    );
    loop {
        // or profcfs, ps, whatever..
        match kill(parent_pid, None) {
            Ok(_) => {
                std::thread::sleep(Duration::from_millis(500));
            },
            Err(_) => {
                println!(
                    "[executive {}] parent {} died, cleaning up and exiting",
                    pid, parent_pid
                );
                break;
            },
        }
    }

    println!(
        "[executive {}] killing process-compose (pid {})...",
        pid,
        pc_child.id()
    );
    pc_child.kill().ok();
    pc_child.wait().ok();

    println!("[executive {}] sleeping before exit...", pid);
    std::thread::sleep(Duration::from_secs(5));

    println!("[executive {}] exiting", pid);
}

fn activate() {
    let data = ActivateData {
        env: "/nix/store/xxx-env".to_string(),
        shell: "/bin/bash".to_string(),
        mode: "dev".to_string(),
    };

    let parent_pid = getpid();

    match unsafe { fork() }.expect("fork failed") {
        ForkResult::Child => {
            executive(data.clone(), parent_pid);
            std::process::exit(0);
        },
        ForkResult::Parent { child } => {
            println!("[activate {}] forked executive child {}", parent_pid, child);

            let mut signals = Signals::new([SIGUSR1]).expect("failed to create Signals");
            for signal in signals.forever() {
                if signal == SIGUSR1 {
                    println!(
                        "[activate {}] received SIGUSR1 - activation ready",
                        parent_pid
                    );
                    break;
                }
            }

            println!(
                "[activate {}] executive child {} still running in background",
                parent_pid, child
            );
            println!("[activate {}] exec-ing into bash...", parent_pid);

            let err = Command::new("/bin/bash")
                .arg("-c")
                // .arg("echo '[activate bash] this is a shell'")
                .arg("echo '[activate bash] this is a shell'; exec /bin/bash")
                .exec();

            panic!("exec failed: {}", err);
        },
    }
}

fn main() {
    activate();
}
