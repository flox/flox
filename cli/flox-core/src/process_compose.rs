//! Shared process-compose constants and liveness.
//!
//! This answers whether a manager *responds*, which is not the same question
//! as whether one *exists*.

use std::io::{Read, Write};
use std::os::unix::io::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
use tracing::debug;

/// Name of the never-exit service that keeps process-compose running.
pub const PROCESS_NEVER_EXIT_NAME: &str = "flox_never_exit";

/// How long a manager has to accept a connection and answer it.
///
/// Reached only when something is wrong: a healthy manager answers in well
/// under a millisecond.
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(250);

/// Most we will read from the manager before giving up on it.
const RESPONSE_LIMIT: u64 = 64 * 1024;

/// Whether a service manager is answering on `socket`.
///
/// Asks `process-compose`'s own liveness endpoint rather than inferring from
/// the socket, which only ever proves something is *bound*. A manager that has
/// stopped servicing requests reads the same here on every platform, where
/// connecting reported it differently on each.
///
/// It reads the same as a manager that is gone, too. Nothing asked over the
/// socket can separate those, and nothing tries.
pub fn manager_responds(socket: &Path) -> bool {
    let Some(mut stream) = connect_bounded(socket) else {
        return false;
    };
    if stream.set_read_timeout(Some(RESPONSE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(RESPONSE_TIMEOUT)).is_err()
    {
        return false;
    }

    let request = "GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if let Err(err) = stream.write_all(request.as_bytes()) {
        debug!(?socket, %err, "could not reach the service manager");
        return false;
    }

    // The status line is the whole answer, so the response is never parsed:
    // no header/body split, and nothing here needs to know how the manager
    // frames what follows it. Bounded by the read timeout, so one that accepts
    // and then goes quiet cannot hang the caller, and by the cap, so a
    // confused peer cannot stream at us.
    let mut response = String::new();
    if let Err(err) = stream.take(RESPONSE_LIMIT).read_to_string(&mut response) {
        debug!(?socket, %err, "the service manager did not answer");
        return false;
    }

    response.starts_with("HTTP/1.1 200")
}

/// Connect without waiting indefinitely.
///
/// A blocking `connect` to a socket whose listen backlog is full waits forever
/// on Linux. Anything that is not a completed connection — no socket, nothing
/// listening, a path we cannot reach — is simply "not answering".
fn connect_bounded(path: &Path) -> Option<UnixStream> {
    let addr = UnixAddr::new(path).ok()?;
    let sock = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )
    .ok()?;
    // SOCK_NONBLOCK is not portable to darwin, so set it after the fact.
    fcntl(&sock, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).ok()?;

    match connect(std::os::unix::io::AsRawFd::as_raw_fd(&sock), &addr) {
        Ok(()) => {},
        Err(Errno::EINPROGRESS) => {
            let mut fds = [PollFd::new(sock.as_fd(), PollFlags::POLLOUT)];
            let timeout = PollTimeout::try_from(RESPONSE_TIMEOUT).ok()?;
            match poll(&mut fds, timeout) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {},
            }
        },
        Err(_) => return None,
    }

    let stream = UnixStream::from(sock);
    // Back to blocking so the read and write timeouts above govern, rather
    // than every call returning EWOULDBLOCK.
    stream.set_nonblocking(false).ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;

    use super::*;

    /// macOS caps socket paths near 104 bytes, below what a `TempDir` under a
    /// long `TMPDIR` produces.
    fn short_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("pcsock")
            .tempdir_in("/tmp")
            .unwrap()
    }

    /// Stand in for a manager: answer `GET /live` once, then stop.
    ///
    /// Reads the request before replying, as any real server does. Closing a
    /// socket with unread data in its receive buffer sends an RST on Linux,
    /// which discards the reply that was just written.
    fn serve_live(listener: UnixListener, status: &'static str) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            }
        })
    }

    #[test]
    fn a_manager_that_answers_responds() {
        let dir = short_tempdir();
        let path = dir.path().join("live.sock");
        let serving = serve_live(UnixListener::bind(&path).unwrap(), "200 OK");

        assert!(manager_responds(&path));
        serving.join().unwrap();
    }

    /// The reported bug: the file outlives its owner.
    #[test]
    fn a_socket_whose_listener_is_gone_does_not_respond() {
        let dir = short_tempdir();
        let path = dir.path().join("stale.sock");
        drop(UnixListener::bind(&path).unwrap());

        assert!(path.exists(), "the file outlives its listener");
        assert!(!manager_responds(&path));
    }

    #[test]
    fn nothing_at_the_path_does_not_respond() {
        let dir = short_tempdir();
        assert!(!manager_responds(&dir.path().join("absent.sock")));
    }

    #[test]
    fn a_regular_file_does_not_respond() {
        let dir = short_tempdir();
        let path = dir.path().join("notasocket");
        std::fs::write(&path, "").unwrap();

        assert!(!manager_responds(&path));
    }

    /// Something bound and accepting but not process-compose is not a manager.
    #[test]
    fn a_listener_that_is_not_a_manager_does_not_respond() {
        let dir = short_tempdir();
        let path = dir.path().join("other.sock");
        let serving = serve_live(UnixListener::bind(&path).unwrap(), "404 Not Found");

        assert!(!manager_responds(&path));
        serving.join().unwrap();
    }

    /// A manager that accepts and then goes quiet is what connecting alone
    /// could never detect: bounded here rather than hanging.
    #[test]
    fn a_listener_that_never_answers_does_not_respond() {
        let dir = short_tempdir();
        let path = dir.path().join("wedged.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let holding = std::thread::spawn(move || listener.accept().map(|(s, _)| s));

        let started = std::time::Instant::now();
        assert!(!manager_responds(&path));
        assert!(
            started.elapsed() < RESPONSE_TIMEOUT * 4,
            "must be bounded, took {:?}",
            started.elapsed()
        );
        drop(holding.join().unwrap());
    }
}
