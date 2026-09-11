//! Shared process-compose constants and socket inspection.

use std::io::ErrorKind;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::io::{AsFd, AsRawFd};
use std::path::Path;
use std::time::Duration;

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{
    AddressFamily,
    SockFlag,
    SockType,
    UnixAddr,
    connect,
    getsockopt,
    socket,
    sockopt,
};
use tracing::debug;

/// Name of the never-exit service that keeps process-compose running.
pub const PROCESS_NEVER_EXIT_NAME: &str = "flox_never_exit";

/// How long to let a connection attempt sit before deciding the manager is
/// bound but not accepting.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

/// How many times to retry a refused connection before calling it stale.
///
/// A full listen backlog is refused on BSD and is indistinguishable from a
/// socket nobody owns, so retry: a backlog drains, a dead socket never does.
const REFUSED_ATTEMPTS: u32 = 3;

/// What is at a `process-compose` socket path.
///
/// `process-compose` unlinks its socket only on a clean shutdown, so the
/// file's presence proves nothing. Connecting is the only way to tell a
/// running manager from one that was killed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketState {
    /// Nothing at the path.
    Missing,
    /// A socket with a process listening on it.
    Live,
    /// A socket whose listener is gone. Harmless: the next `process-compose`
    /// to bind the path unlinks it and rebinds.
    ///
    /// A listener whose backlog stays full is refused on BSD and is not
    /// distinguishable from this by connecting. [REFUSED_ATTEMPTS] covers a
    /// backlog that drains; one that never does reads as stale, and such a
    /// manager is unreachable either way.
    Stale,
    /// Anything else. Never acted upon.
    Unknown,
}

impl SocketState {
    pub fn is_live(&self) -> bool {
        matches!(self, SocketState::Live)
    }
}

/// Classify what is at `path` by trying to connect to it.
///
/// Deliberately total: a path we cannot classify is [SocketState::Unknown] and
/// is left alone.
pub fn socket_state(path: &Path) -> SocketState {
    let file_type = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type(),
        Err(err) if err.kind() == ErrorKind::NotFound => return SocketState::Missing,
        Err(err) => {
            debug!(?path, %err, "could not stat services socket");
            return SocketState::Unknown;
        },
    };

    if !file_type.is_socket() {
        debug!(?path, "services socket path is not a socket");
        return SocketState::Unknown;
    }

    for attempt in 1..=REFUSED_ATTEMPTS {
        match try_connect(path) {
            Ok(()) => return SocketState::Live,
            // Something is bound but is not taking connections right now. Not
            // stale — spawning a second manager over it would strand this one.
            Err(Errno::EAGAIN) | Err(Errno::EINPROGRESS) => return SocketState::Live,
            Err(Errno::ECONNREFUSED) if attempt < REFUSED_ATTEMPTS => {
                std::thread::sleep(Duration::from_millis(20));
            },
            Err(Errno::ECONNREFUSED) => return SocketState::Stale,
            Err(err) => {
                debug!(?path, %err, "could not connect to services socket");
                return SocketState::Unknown;
            },
        }
    }

    SocketState::Stale
}

/// One non-blocking connection attempt.
///
/// Non-blocking because a blocking `connect` to a socket whose backlog is full
/// waits indefinitely on Linux, and this runs on interactive paths.
fn try_connect(path: &Path) -> Result<(), Errno> {
    let addr = UnixAddr::new(path)?;
    let sock = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )?;
    // SOCK_NONBLOCK is not portable to darwin, so set it after the fact.
    fcntl(&sock, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;

    match connect(sock.as_raw_fd(), &addr) {
        Ok(()) => return Ok(()),
        Err(Errno::EINPROGRESS) => {},
        Err(err) => return Err(err),
    }

    let mut fds = [PollFd::new(sock.as_fd(), PollFlags::POLLOUT)];
    let timeout = PollTimeout::try_from(CONNECT_TIMEOUT).unwrap_or(PollTimeout::MAX);
    match poll(&mut fds, timeout)? {
        // Nothing accepted us in time: bound, but not answering.
        0 => Err(Errno::EAGAIN),
        _ => match getsockopt(&sock, sockopt::SocketError)? {
            0 => Ok(()),
            err => Err(Errno::from_raw(err)),
        },
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn nothing_at_the_path_is_missing() {
        let dir = short_tempdir();
        assert_eq!(
            socket_state(&dir.path().join("absent.sock")),
            SocketState::Missing
        );
    }

    #[test]
    fn a_bound_socket_is_live() {
        let dir = short_tempdir();
        let path = dir.path().join("live.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        assert_eq!(socket_state(&path), SocketState::Live);
        assert!(socket_state(&path).is_live());
    }

    #[test]
    fn a_socket_whose_listener_is_gone_is_stale() {
        let dir = short_tempdir();
        let path = dir.path().join("stale.sock");
        let listener = UnixListener::bind(&path).unwrap();
        drop(listener);

        assert!(path.exists(), "the file outlives its listener");
        assert_eq!(socket_state(&path), SocketState::Stale);
        assert!(!socket_state(&path).is_live());
    }

    /// A manager that is slow to call `accept` is still a manager. Pending
    /// connections sit in the backlog and the probe completes, so being busy
    /// never reads as being gone.
    #[test]
    fn a_bound_socket_that_has_not_accepted_yet_is_live() {
        let dir = short_tempdir();
        let path = dir.path().join("busy.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        let _pending: Vec<_> = (0..4)
            .filter_map(|_| std::os::unix::net::UnixStream::connect(&path).ok())
            .collect();

        assert_eq!(socket_state(&path), SocketState::Live);
    }

    #[test]
    fn a_regular_file_is_unknown() {
        let dir = short_tempdir();
        let path = dir.path().join("notasocket");
        std::fs::write(&path, "").unwrap();

        assert_eq!(socket_state(&path), SocketState::Unknown);
    }

    #[test]
    fn a_path_under_a_missing_directory_is_missing() {
        let path = Path::new("/does_not_exist_dir/nested/services.sock");
        assert_eq!(socket_state(path), SocketState::Missing);
    }
}
