use config::{Config, Map, Value};

use anyhow::Result;
use lazy_static::lazy_static;
use tokio::sync::RwLock;

lazy_static! {
    pub static ref CONFIG: RwLock<Config> = {
        let config = if !std::path::Path::new("./flox.toml").exists() {
            Config::builder().add_source(config::Environment::with_prefix("FLOX"))
        } else {
            let src = config::File::with_name("flox.toml");
            Config::builder()
                .add_source(config::Environment::with_prefix("FLOX"))
                .add_source(src)
        };

        RwLock::new(config.build().unwrap())
    };
}

async fn dump_config() -> Result<()> {
    let config = CONFIG.write().await.clone();

    println!("{:?}", config.try_deserialize::<Map<String, Value>>()?);

    Ok(())
}
