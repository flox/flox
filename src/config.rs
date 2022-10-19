
use config::{Config, FileSourceFile, Map, Value};
use serde::Deserialize;
use tokio::sync::RwLock;
use anyhow::{Result};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref CONFIG : RwLock<Config> = {
        let src = config::File::with_name("flox.toml");
        RwLock::new(Config::builder().add_source(src)
        .add_source(config::Environment::with_prefix("FLOX"))
        .build().unwrap())
    };
}
