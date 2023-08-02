use anyhow::{self, bail, Context, Result};
use flox_types::catalog::cache::Cache as CatalogCache;
use flox_types::catalog::cache::{CacheMeta, Narinfo, SubstituterUrl};
use flox_types::catalog::DerivationPath;
use futures::future::join_all;
use futures::prelude::*;
use futures::stream;

use log::warn;
use serde::{Serialize, Deserialize};
use serde_json::{self, Deserializer, Value};

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io;

use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use log::{debug, error, info};
use par_stream::prelude::ParStreamExt as _;
use tokio::process::Command;

use clap::Parser;

mod cache;
use cache::{Cache, CacheItem};

/// Simple program to greet a person
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long = "substituter", default_value = "https://cache.nixos.org")]
    substituters: Vec<SubstituterUrl>,

    #[clap(short, long)]
    cache_db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args = Args::parse();

    // Create a cache from specified cache file or only in-memory
    let fetch_cache = if let Some(path) = &args.cache_db {
        if !(Path::new(path).exists()) {
            info!("Cache file at `{path:?}` not found, starting with in-memory cache");
            Cache::default()
        } else {
            let reader = io::BufReader::new(File::open(path)?);
            serde_json::from_reader(reader)?
        }
    } else {
        info!("Using in-memory cache");
        Cache::default()
    };

    // Prepare shared data
    let args = Arc::new(args);
    let fetch_cache = Arc::new(RwLock::new(fetch_cache));

    // read json object stream from stdin process and send send result to stdout
    process_json_stream(args.clone(), fetch_cache.clone()).await?;

    // update cache file
    if let Some(path) = &args.cache_db {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        let writer = io::BufWriter::new(file);
        serde_json::to_writer(writer, &*fetch_cache.read().unwrap())
            .with_context(|| "Failed writing cache")?;
    }

    Ok(())
}

async fn process_json_stream(args: Arc<Args>, fetch_cache: Arc<RwLock<Cache>>) -> Result<()> {
    let stdin = io::stdin();
    let deserializer_iter = Deserializer::from_reader(stdin).into_iter().filter_map(
        |res: Result<CatalogEntry, serde_json::Error>| {
            if let Some(error) = res.as_ref().err() {
                error!("Deserialization Error: {error}");
                return None;
            }
            res.ok()
        },
    );

    let json_stream = stream::iter(deserializer_iter);

    let _x = json_stream
        .par_map_unordered(None, move |item| {
            let args = args.clone();
            let fetch_cache = fetch_cache.clone();
            move || fetch_substituters(args, fetch_cache, item)
        })
        .for_each(|item| async {
            match item.await {
                Ok(item) => serde_json::to_writer(io::stdout(), &item).unwrap(),
                Err(e) => error!("Error while fetching: {e}"),
            }
            ()
        })
        .await;

    Ok(())
}

async fn fetch_substituters(
    args: Arc<Args>,
    fetch_cache: Arc<RwLock<Cache>>,
    mut item: CatalogEntry,
) -> Result<CatalogEntry> {
    let fetches = args
        .substituters
        .iter()
        .map(|substituter| fetch_substituter(substituter, fetch_cache.clone(), &item));

    let cache_metas = join_all(fetches).await;

    let cache = item.cache.get_or_insert(CatalogCache::default());
    for cache_meta in cache_metas.into_iter() {
        match cache_meta {
            Ok(meta) => cache.add(meta),
            Err(e) => error!("{e}"),
        }
    }

    Ok(item)
}

async fn fetch_substituter(
    substituter: &SubstituterUrl,
    fetch_cache: Arc<RwLock<Cache>>,
    item: &CatalogEntry,
) -> Result<CacheMeta> {
    info!("Querying {substituter}");

    // Lookup store paths in the cache separate uncached ones
    let (cached, uncached): (
        Vec<(&DerivationPath, Option<Narinfo>)>,
        Vec<(&DerivationPath, Option<Narinfo>)>,
    ) = item
        .element
        .store_paths
        .iter()
        .map(|drv| {
            let drv_key = (*drv).to_owned();
            (
                drv,
                fetch_cache
                    .read()
                    .unwrap()
                    .get(&(substituter.to_string(), drv_key))
                    .cloned()
                    .map(|ci| ci.narinfo),
            )
        })
        .partition(|(_, opt)| opt.is_some());

    // Check wheter uncached
    let uncached = uncached.iter().map(|(drv, _)| *drv).collect::<Vec<_>>();
    let narinfo: Vec<Narinfo> = if uncached.is_empty() {
        info!("All inputs cached");
        Vec::new()
    } else {
        let mut command = make_command(&substituter, uncached);

        let output = command.output().await?;

        if !ExitStatus::success(&output.status) {
            // TODO: error handling
            bail!("nix path-info: {}", String::from_utf8_lossy(&output.stderr))
        }
        if !output.stderr.is_empty() {
            warn!("nix path-info: {}", String::from_utf8_lossy(&output.stderr))
        }
        serde_json::from_slice(&output.stdout)?
    };

    let (mut hits, misses): (Vec<&Narinfo>, Vec<&Narinfo>) =
        narinfo.iter().partition(|info| info.valid);

    if !misses.is_empty() {
        info!(
            "cache misses: {:?}",
            misses
                .iter()
                .map(|info| info.path.clone())
                .collect::<Vec<_>>()
        );
    } else {
        hits.extend(cached.iter().map(|(_, info)| info.as_ref().unwrap()));
        let mut cache = fetch_cache.write().unwrap();
        hits.iter().cloned().for_each(|info| {
            cache.insert(
                (substituter.to_string(), info.path.to_owned()),
                CacheItem {
                    ts: SystemTime::now(),
                    narinfo: info.clone(),
                },
            );
        });
    }

    Ok(CacheMeta {
        cache_url: substituter.to_owned(),
        narinfo,
        _other: Default::default(),
    })
}

fn make_command(
    substituter: &SubstituterUrl,
    derivation: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Command {
    let mut command = Command::new("nix");
    command
        .arg("path-info")
        .arg("--json")
        .args(&["--eval-store", "auto"])
        .args(&["--store", substituter.as_ref()]) // select custom substituter is specified
        .args(derivation.into_iter());

    debug!("{:?}", command.as_std());

    command
}


/// TEMPORARY, url tyoes have to be fixed in flox tye using runix types
#[derive(Serialize, Deserialize)]
struct CatalogEntry {
    element: flox_types::catalog::Element,
    cache: Option<CatalogCache>,
    #[serde(flatten)]
    _other: HashMap<String, Value>
}
