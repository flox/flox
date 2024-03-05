use std::io::Stderr;
use std::sync::Mutex;

use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod didyoumean;
pub mod errors;
pub mod init;
pub mod message;
pub mod metrics;
pub mod openers;
pub mod search;

pub static TERMINAL_STDERR: Lazy<Mutex<Stderr>> = Lazy::new(|| Mutex::new(std::io::stderr()));
