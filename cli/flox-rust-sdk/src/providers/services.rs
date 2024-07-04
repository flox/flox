use std::env;

use once_cell::sync::Lazy;

pub const SERVICES_ENV_VAR: &str = "FLOX_FEATURES_SERVICES";
pub static PROCESS_COMPOSE_BIN: Lazy<String> = Lazy::new(|| {
    env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});
