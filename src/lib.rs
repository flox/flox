
use clap::Parser;

#[path="providers/flox.rs"]
mod floxprovider;
#[path="providers/github.rs"]
mod githubprovider;

mod models;

pub mod environment;
