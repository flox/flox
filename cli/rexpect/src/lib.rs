//! The main crate of Rexpect
//!
//! # Overview
//!
//! Rexpect is a loose port of [pexpect](https://pexpect.readthedocs.io/en/stable/)
//! which itself is inspired by Don Libe's expect.
//!
//! It's main components (depending on your need you can use either of those)
//!
//! - [session](session/index.html): automate stuff in Rust
//! - [reader](reader/index.html): a non-blocking reader with buffering, matching on
//!   strings/regex/...
//! - [process](process/index.html): spawn a process in a pty
//!
//! # Basic example
//!
//! ```no_run
//!
//! use rexpect::spawn;
//! use rexpect::error::Error;
//!
//! fn main() -> Result<(), Error> {
//!     let mut p = spawn("ftp speedtest.tele2.net", Some(2000))?;
//!     p.exp_regex("Name \\(.*\\):")?;
//!     p.send_line("anonymous")?;
//!     p.exp_string("Password")?;
//!     p.send_line("test")?;
//!     p.exp_string("ftp>")?;
//!     p.send_line("cd upload")?;
//!     p.exp_string("successfully changed.\r\nftp>")?;
//!     p.send_line("pwd")?;
//!     p.exp_regex("[0-9]+ \"/upload\"")?;
//!     p.send_line("exit")?;
//!     p.exp_eof()?;
//!     Ok(())
//! }
//! ```
//!
//! # Example with bash
//!
//! Tip: try the chain of commands first in a bash session.
//! The tricky thing is to get the `wait_for_prompt` right.
//! What `wait_for_prompt` actually does is seeking to the next
//! visible prompt. If you forgot to call this once your next call to
//! `wait_for_prompt` comes out of sync and you're seeking to a prompt
//! printed "above" the last `execute()`.
//!
//! ```no_run
//! use rexpect::spawn_bash;
//! use rexpect::error::Error;
//!
//! fn main() -> Result<(), Error> {
//!     let mut p = spawn_bash(Some(30_000))?;
//!     p.execute("ping 8.8.8.8", "bytes of data")?;
//!     p.send_control('z')?;
//!     p.wait_for_prompt()?;
//!     p.execute("bg", "suspended")?;
//!     p.send_line("sleep 1")?;
//!     p.wait_for_prompt()?;
//!     p.execute("fg", "continued")?;
//!     p.send_control('c')?;
//!     p.exp_string("packet loss")?;
//!     Ok(())
//! }
//! ```

#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![warn(clippy::print_stderr)]
#![warn(clippy::print_stdout)]

pub mod error;
pub mod process;
pub mod reader;
pub mod session;

pub use reader::ReadUntil;
pub use session::{spawn, spawn_bash, spawn_python, spawn_stream, spawn_with_options};

// include the README.md here to test its doc
#[doc = include_str!("../README.md")]
mod test {}
