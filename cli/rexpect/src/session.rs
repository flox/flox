//! Main module of rexpect: start new process and interact with it

use crate::error::Error; // load error-chain
use crate::process::PtyProcess;
use crate::reader::{NBReader, Regex};
pub use crate::reader::{Options, ReadUntil};
use std::fs::File;
use std::io::prelude::*;
use std::io::LineWriter;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::process::Command;
use tempfile;

pub struct StreamSession<W: Write> {
    pub writer: LineWriter<W>,
    pub reader: NBReader,
}

impl<W: Write> StreamSession<W> {
    pub fn new<R: Read + Send + 'static>(reader: R, writer: W, options: Options) -> Self {
        Self {
            writer: LineWriter::new(writer),
            reader: NBReader::new(reader, options),
        }
    }

    /// sends string and a newline to process
    ///
    /// this is guaranteed to be flushed to the process
    /// returns number of written bytes
    pub fn send_line(&mut self, line: &str) -> Result<usize, Error> {
        let mut len = self.send(line)?;
        len += self.writer.write(b"\n")?;
        Ok(len)
    }

    /// Send string to process. As stdin of the process is most likely buffered, you'd
    /// need to call `flush()` after `send()` to make the process actually see your input.
    ///
    /// Returns number of written bytes
    pub fn send(&mut self, s: &str) -> Result<usize, Error> {
        self.writer.write(s.as_bytes()).map_err(Error::from)
    }

    /// Send a control code to the running process and consume resulting output line
    /// (which is empty because echo is off)
    ///
    /// E.g. `send_control('c')` sends ctrl-c. Upper/smaller case does not matter.
    pub fn send_control(&mut self, c: char) -> Result<(), Error> {
        let code = match c {
            'a'..='z' => c as u8 + 1 - b'a',
            'A'..='Z' => c as u8 + 1 - b'A',
            '[' => 27,
            '\\' => 28,
            ']' => 29,
            '^' => 30,
            '_' => 31,
            _ => return Err(Error::SendControlError(c)),
        };
        self.writer.write_all(&[code])?;
        // stdout is line buffered, so needs a flush
        self.writer.flush()?;
        Ok(())
    }

    /// Make sure all bytes written via `send()` are sent to the process
    pub fn flush(&mut self) -> Result<(), Error> {
        self.writer.flush().map_err(Error::from)
    }

    /// Read one line (blocking!) and return line without the newline
    /// (waits until \n is in the output fetches the line and removes \r at the end if present)
    pub fn read_line(&mut self) -> Result<String, Error> {
        match self.exp(&ReadUntil::String('\n'.to_string())) {
            Ok((mut line, _)) => {
                if line.ends_with('\r') {
                    line.pop().expect("this never happens");
                }
                Ok(line)
            }
            Err(e) => Err(e),
        }
    }

    /// Return `Some(c)` if a char is ready in the stdout stream of the process, return `None`
    /// otherwise. This is non-blocking.
    pub fn try_read(&mut self) -> Option<char> {
        self.reader.try_read()
    }

    // wrapper around reader::read_until to give more context for errors
    fn exp(&mut self, needle: &ReadUntil) -> Result<(String, String), Error> {
        self.reader.read_until(needle)
    }

    /// Wait until we see EOF (i.e. child process has terminated)
    /// Return all the yet unread output
    pub fn exp_eof(&mut self) -> Result<String, Error> {
        self.exp(&ReadUntil::EOF).map(|(_, s)| s)
    }

    /// Wait until provided regex is seen on stdout of child process.
    /// Return a tuple:
    /// 1. the yet unread output
    /// 2. the matched regex
    ///
    /// Note that `exp_regex("^foo")` matches the start of the yet consumed output.
    /// For matching the start of the line use `exp_regex("\nfoo")`
    pub fn exp_regex(&mut self, regex: &str) -> Result<(String, String), Error> {
        self.exp(&ReadUntil::Regex(Regex::new(regex)?))
    }

    /// Wait until provided string is seen on stdout of child process.
    /// Return the yet unread output (without the matched string)
    pub fn exp_string(&mut self, needle: &str) -> Result<String, Error> {
        self.exp(&ReadUntil::String(needle.to_owned()))
            .map(|(s, _)| s)
    }

    /// Wait until provided char is seen on stdout of child process.
    /// Return the yet unread output (without the matched char)
    pub fn exp_char(&mut self, needle: char) -> Result<String, Error> {
        self.exp(&ReadUntil::String(needle.to_string()))
            .map(|(s, _)| s)
    }

    /// Wait until any of the provided needles is found.
    ///
    /// Return a tuple with:
    /// 1. the yet unread string, without the matching needle (empty in case of EOF and `NBytes`)
    /// 2. the matched string
    ///
    /// # Example:
    ///
    /// ```
    /// use rexpect::{spawn, ReadUntil};
    /// # use rexpect::error::Error;
    ///
    /// # fn main() {
    ///     # || -> Result<(), Error> {
    /// let mut s = spawn("cat", Some(1000))?;
    /// s.send_line("hello, polly!")?;
    /// s.exp_any(vec![ReadUntil::String("hello".into()),
    ///                ReadUntil::EOF])?;
    ///         # Ok(())
    ///     # }().expect("test failed");
    /// # }
    /// ```
    pub fn exp_any(&mut self, needles: Vec<ReadUntil>) -> Result<(String, String), Error> {
        self.exp(&ReadUntil::Any(needles))
    }
}
/// Interact with a process with read/write/signals, etc.
#[allow(dead_code)]
pub struct PtySession {
    pub process: PtyProcess,
    pub stream: StreamSession<File>,
}

// make StreamSession's methods available directly
impl Deref for PtySession {
    type Target = StreamSession<File>;
    fn deref(&self) -> &StreamSession<File> {
        &self.stream
    }
}

impl DerefMut for PtySession {
    fn deref_mut(&mut self) -> &mut StreamSession<File> {
        &mut self.stream
    }
}

/// Start a process in a tty session, write and read from it
///
/// # Example
///
/// ```
///
/// use rexpect::spawn;
/// # use rexpect::error::Error;
///
/// # fn main() {
///     # || -> Result<(), Error> {
/// let mut s = spawn("cat", Some(1000))?;
/// s.send_line("hello, polly!")?;
/// let line = s.read_line()?;
/// assert_eq!("hello, polly!", line);
///         # Ok(())
///     # }().expect("test failed");
/// # }
/// ```
impl PtySession {
    fn new(process: PtyProcess, options: Options) -> Result<Self, Error> {
        let f = process.get_file_handle()?;
        let reader = f.try_clone()?;
        let stream = StreamSession::new(reader, f, options);
        Ok(Self { process, stream })
    }
}

/// Turn e.g. "prog arg1 arg2" into ["prog", "arg1", "arg2"]
/// Also takes care of single and double quotes
fn tokenize_command(program: &str) -> Result<Vec<String>, Error> {
    comma::parse_command(program).ok_or(Error::BadProgramArguments)
}

/// Start command in background in a pty session (pty fork) and return a struct
/// with writer and buffered reader (for unblocking reads).
///
/// #Arguments:
///
/// - `program`: This is split at spaces and turned into a `process::Command`
///   if you wish more control over this, use `spawn_command`
/// - `timeout`: If Some: all `exp_*` commands time out after x milliseconds, if None: never times
///   out.
///   It's highly recommended to put a timeout there, as otherwise in case of
///   a problem the program just hangs instead of exiting with an
///   error message indicating where it stopped.
///   For automation 30'000 (30s, the default in pexpect) is a good value.
pub fn spawn(program: &str, timeout_ms: Option<u64>) -> Result<PtySession, Error> {
    if program.is_empty() {
        return Err(Error::EmptyProgramName);
    }

    let mut parts = tokenize_command(program)?;
    let prog = parts.remove(0);
    let mut command = Command::new(prog);
    command.args(parts);
    spawn_command(command, timeout_ms)
}

/// See `spawn`
pub fn spawn_command(command: Command, timeout_ms: Option<u64>) -> Result<PtySession, Error> {
    spawn_with_options(
        command,
        Options {
            timeout_ms,
            strip_ansi_escape_codes: false,
        },
    )
}

/// See `spawn`
pub fn spawn_with_options(command: Command, options: Options) -> Result<PtySession, Error> {
    #[cfg(feature = "which")]
    {
        let _ = which::which(command.get_program())?;
    }
    let mut process = PtyProcess::new(command)?;
    process.set_kill_timeout(options.timeout_ms);

    PtySession::new(process, options)
}

/// A repl session: e.g. bash or the python shell:
/// You have a prompt where a user inputs commands and the shell
/// executes it and writes some output
pub struct PtyReplSession {
    /// the prompt, used for `wait_for_prompt`, e.g. ">>> " for python
    pub prompt: String,

    /// the `pty_session` you prepared before (initiating the shell, maybe set a custom prompt, etc.)
    /// see `spawn_bash` for an example
    pub pty_session: PtySession,

    /// if set, then the `quit_command` is called when this object is dropped
    /// you need to provide this if the shell you're testing is not killed by just sending
    /// SIGTERM
    pub quit_command: Option<String>,

    /// set this to true if the repl has echo on (i.e. sends user input to stdout)
    /// although echo is set off at pty fork (see `PtyProcess::new`) a few repls still
    /// seem to be able to send output. You may need to try with true first, and if
    /// tests fail set this to false.
    pub echo_on: bool,
}

impl PtyReplSession {
    pub fn wait_for_prompt(&mut self) -> Result<String, Error> {
        self.pty_session.exp_string(&self.prompt)
    }

    /// Send cmd to repl and:
    /// 1. wait for the cmd to be echoed (if `echo_on == true`)
    /// 2. wait for the ready string being present
    ///
    /// Q: Why can't I just do `send_line` and immediately continue?
    /// A: Executing a command in e.g. bash causes a fork. If the Unix kernel chooses the
    ///    parent process (bash) to go first and the bash process sends e.g. Ctrl-C then the
    ///    Ctrl-C goes to nirvana.
    ///    The only way to prevent this situation is to wait for a ready string being present
    ///    in the output.
    ///
    /// Another safe way to tackle this problem is to use `send_line()` and `wait_for_prompt()`
    ///
    /// # Example:
    ///
    /// ```
    /// use rexpect::spawn_bash;
    /// # use rexpect::error::Error;
    ///
    /// # fn main() {
    ///     # || -> Result<(), Error> {
    /// let mut p = spawn_bash(Some(1000))?;
    /// p.execute("cat <(echo ready) -", "ready")?;
    /// p.send_line("hans")?;
    /// p.exp_string("hans")?;
    ///         # Ok(())
    ///     # }().expect("test failed");
    /// # }
    /// ```
    pub fn execute(&mut self, cmd: &str, ready_regex: &str) -> Result<(), Error> {
        self.send_line(cmd)?;
        if self.echo_on {
            self.exp_string(cmd)?;
        }
        self.exp_regex(ready_regex)?;
        Ok(())
    }

    /// send line to repl (and flush output) and then, if `echo_on=true` wait for the
    /// input to appear.
    /// Return: number of bytes written
    pub fn send_line(&mut self, line: &str) -> Result<usize, Error> {
        let bytes_written = self.pty_session.send_line(line)?;
        if self.echo_on {
            self.exp_string(line)?;
        }
        Ok(bytes_written)
    }
}

// make PtySession's methods available directly
impl Deref for PtyReplSession {
    type Target = PtySession;
    fn deref(&self) -> &PtySession {
        &self.pty_session
    }
}

impl DerefMut for PtyReplSession {
    fn deref_mut(&mut self) -> &mut PtySession {
        &mut self.pty_session
    }
}

impl Drop for PtyReplSession {
    /// for e.g. bash we *need* to run `quit` at the end.
    /// if we leave that out, `PtyProcess` would try to kill the bash
    /// which would not work, as a SIGTERM is not enough to kill bash
    fn drop(&mut self) {
        if let Some(ref cmd) = self.quit_command {
            self.pty_session
                .send_line(cmd)
                .unwrap_or_else(|_| panic!("could not run `{cmd}` on child process"));
        }
    }
}

/// Spawn bash in a pty session, run programs and expect output
///
///
/// The difference to `spawn` and `spawn_command` is:
///
/// - `spawn_bash` starts bash with a custom rcfile which guarantees
///   a certain prompt
/// - the `PtyBashSession` also provides `wait_for_prompt` and `execute`
///
/// timeout: the duration until which `exp_*` returns a timeout error, or None
/// additionally, when dropping the bash prompt while bash is still blocked by a program
/// (e.g. `sleep 9999`) then the timeout is used as a timeout before a `kill -9` is issued
/// at the bash command. Use a timeout whenever possible because it makes
/// debugging a lot easier (otherwise the program just hangs and you
/// don't know where)
///
/// bash is started with echo off. That means you don't need to "read back"
/// what you wrote to bash. But what you need to do is a `wait_for_prompt`
/// after a process finished.
///
/// Also: if you start a program you should use `execute` and not `send_line`.
///
/// For an example see the README
pub fn spawn_bash(timeout: Option<u64>) -> Result<PtyReplSession, Error> {
    // unfortunately working with a temporary tmpfile is the only
    // way to guarantee that we are "in step" with the prompt
    // all other attempts were futile, especially since we cannot
    // wait for the first prompt since we don't know what .bashrc
    // would set as PS1 and we cannot know when is the right time
    // to set the new PS1
    let mut rcfile = tempfile::NamedTempFile::new()?;
    let _ = rcfile.write(
        b"include () { [[ -f \"$1\" ]] && source \"$1\"; }\n\
                  include /etc/bash.bashrc\n\
                  include ~/.bashrc\n\
                  PS1=\"~~~~\"\n\
                  unset PROMPT_COMMAND\n",
    )?;
    let mut c = Command::new("bash");
    c.args([
        "--rcfile",
        rcfile.path().to_str().unwrap_or("temp file does not exist"),
    ]);
    spawn_command(c, timeout).and_then(|p| {
        let new_prompt = "[REXPECT_PROMPT>";
        let mut pb = PtyReplSession {
            prompt: new_prompt.to_owned(),
            pty_session: p,
            quit_command: Some("exit".to_owned()),
            echo_on: false,
        };
        pb.send_line(
            r#"PS1="~~~~" && unset PROMPT_COMMAND && bind 'set enable-bracketed-paste off'"#,
        )?;
        pb.exp_string("~~~~")?;
        let ps1 = format!("PS1='{new_prompt}'");
        pb.send_line(&ps1)?;
        // pb.exp_string("~~~~")?;
        // rcfile.close()?;
        // let ps1 = format!("PS1='{new_prompt}'");
        // pb.send_line(&ps1)?;
        // wait until the new prompt appears
        pb.wait_for_prompt()?;
        Ok(pb)
    })
}

/// Spawn bash in a pty session, run programs and expect output
///
///
/// The difference to `spawn` and `spawn_command` is:
///
/// - `spawn_bash` starts bash with a custom rcfile which guarantees
///   a certain prompt
/// - the `PtyBashSession` also provides `wait_for_prompt` and `execute`
///
/// timeout: the duration until which `exp_*` returns a timeout error, or None
/// additionally, when dropping the bash prompt while bash is still blocked by a program
/// (e.g. `sleep 9999`) then the timeout is used as a timeout before a `kill -9` is issued
/// at the bash command. Use a timeout whenever possible because it makes
/// debugging a lot easier (otherwise the program just hangs and you
/// don't know where)
///
/// bash is started with echo off. That means you don't need to "read back"
/// what you wrote to bash. But what you need to do is a `wait_for_prompt`
/// after a process finished.
///
/// Also: if you start a program you should use `execute` and not `send_line`.
///
/// For an example see the README
pub fn spawn_specific_bash(
    path: impl AsRef<Path>,
    timeout: Option<u64>,
) -> Result<PtyReplSession, Error> {
    let c = Command::new(path.as_ref());
    spawn_command(c, timeout).and_then(|p| {
        let new_prompt = "[REXPECT_PROMPT>";
        let mut pb = PtyReplSession {
            prompt: new_prompt.to_owned(),
            pty_session: p,
            quit_command: Some("exit".to_owned()),
            echo_on: false,
        };
        // This last command is to turn off whatever bracketed paste mode is about
        pb.send_line(
            r#"PS1="~~~~" && unset PROMPT_COMMAND && bind 'set enable-bracketed-paste off'"#,
        )?;
        pb.exp_string("~~~~")?;
        let ps1 = format!("PS1='{new_prompt}'");
        pb.send_line(&ps1)?;
        // wait until the new prompt appears
        pb.wait_for_prompt()?;
        Ok(pb)
    })
}

/// Spawn the python shell
///
/// This is just a proof of concept implementation (and serves for documentation purposes)
pub fn spawn_python(timeout: Option<u64>) -> Result<PtyReplSession, Error> {
    spawn_command(Command::new("python"), timeout).map(|p| PtyReplSession {
        prompt: ">>> ".to_owned(),
        pty_session: p,
        quit_command: Some("exit()".to_owned()),
        echo_on: true,
    })
}

/// Spawn a REPL from a stream
pub fn spawn_stream<R: Read + Send + 'static, W: Write>(
    reader: R,
    writer: W,
    timeout_ms: Option<u64>,
) -> StreamSession<W> {
    StreamSession::new(
        reader,
        writer,
        Options {
            timeout_ms,
            strip_ansi_escape_codes: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_line() -> Result<(), Error> {
        let mut s = spawn("cat", Some(100000))?;
        s.send_line("hans")?;
        assert_eq!("hans", s.read_line()?);
        let should = crate::process::wait::WaitStatus::Signaled(
            s.process.child_pid,
            crate::process::signal::Signal::SIGTERM,
            false,
        );
        assert_eq!(should, s.process.exit()?);
        Ok(())
    }

    #[test]
    fn test_expect_eof_timeout() -> Result<(), Error> {
        let mut p = spawn("sleep 3", Some(1000)).expect("cannot run sleep 3");
        match p.exp_eof() {
            Ok(_) => panic!("should raise Timeout"),
            Err(Error::Timeout { .. }) => {}
            Err(_) => panic!("should raise TimeOut"),
        }
        Ok(())
    }

    #[test]
    fn test_expect_eof_timeout2() {
        let mut p = spawn("sleep 1", Some(1100)).expect("cannot run sleep 1");
        assert!(p.exp_eof().is_ok(), "expected eof");
    }

    #[test]
    fn test_expect_string() -> Result<(), Error> {
        let mut p = spawn("cat", Some(1000)).expect("cannot run cat");
        p.send_line("hello world!")?;
        p.exp_string("hello world!")?;
        p.send_line("hello heaven!")?;
        p.exp_string("hello heaven!")?;
        Ok(())
    }

    #[test]
    fn test_read_string_before() -> Result<(), Error> {
        let mut p = spawn("cat", Some(1000)).expect("cannot run cat");
        p.send_line("lorem ipsum dolor sit amet")?;
        assert_eq!("lorem ipsum dolor sit ", p.exp_string("amet")?);
        Ok(())
    }

    #[test]
    fn test_expect_any() -> Result<(), Error> {
        let mut p = spawn("cat", Some(1000)).expect("cannot run cat");
        p.send_line("Hi")?;
        match p.exp_any(vec![
            ReadUntil::NBytes(3),
            ReadUntil::String("Hi".to_owned()),
        ]) {
            Ok(s) => assert_eq!(("".to_owned(), "Hi".to_owned()), s),
            Err(e) => panic!("got error: {e}"),
        }
        Ok(())
    }

    #[test]
    fn test_expect_empty_command_error() {
        let p = spawn("", Some(1000));
        match p {
            Ok(_) => panic!("should raise an error"),
            Err(Error::EmptyProgramName) => {}
            Err(_) => panic!("should raise EmptyProgramName"),
        }
    }

    #[test]
    fn test_kill_timeout() -> Result<(), Error> {
        let mut p = spawn_bash(Some(1000))?;
        p.execute("cat <(echo ready) -", "ready")?;
        Ok(())
        // p is dropped here and kill is sent immediately to bash
        // Since that is not enough to make bash exit, a kill -9 is sent within 1s (timeout)
    }

    #[test]
    fn test_bash() -> Result<(), Error> {
        let mut p = spawn_bash(Some(1000))?;
        p.send_line("cd /tmp/")?;
        p.wait_for_prompt()?;
        p.send_line("pwd")?;
        assert_eq!("/tmp\r\n", p.wait_for_prompt()?);
        Ok(())
    }

    #[test]
    fn test_bash_control_chars() -> Result<(), Error> {
        let mut p = spawn_bash(Some(1000))?;
        p.execute("cat <(echo ready) -", "ready")?;
        p.send_control('c')?; // abort: SIGINT
        p.wait_for_prompt()?;
        p.execute("cat <(echo ready) -", "ready")?;
        p.send_control('z')?; // suspend:SIGTSTPcon
        p.exp_regex(r"(Stopped|suspended)\s+cat .*")?;
        p.send_line("fg")?;
        p.execute("cat <(echo ready) -", "ready")?;
        p.send_control('c')?;
        Ok(())
    }

    #[test]
    fn test_tokenize_command() {
        let res = tokenize_command("prog arg1 arg2");
        assert_eq!(vec!["prog", "arg1", "arg2"], res.unwrap());

        let res = tokenize_command("prog -k=v");
        assert_eq!(vec!["prog", "-k=v"], res.unwrap());

        let res = tokenize_command("prog 'my text'");
        assert_eq!(vec!["prog", "my text"], res.unwrap());

        let res = tokenize_command(r#"prog "my text""#);
        assert_eq!(vec!["prog", r#"my text"#], res.unwrap());

        let res = tokenize_command(r#"prog "my text with quote'""#);
        assert_eq!(vec!["prog", r#"my text with quote'"#], res.unwrap());

        let res = tokenize_command(r#"prog "my broken text"#);
        assert!(res.is_err());
    }
}
