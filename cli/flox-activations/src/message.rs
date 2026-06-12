use std::fmt::Display;

use flox_core::util::message::{format_error, format_info, format_updated};

pub fn error(v: impl Display) {
    eprintln!("{}", format_error(v));
}

pub fn updated(v: impl Display) {
    eprintln!("{}", format_updated(v));
}

pub fn info(v: impl Display) {
    eprintln!("{}", format_info(v));
}
