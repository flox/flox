use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use async_trait::async_trait;
use flox_rust_sdk::flox::{Flox, FloxInstallable, Floxhub, DEFAULT_FLOXHUB_URL};
use flox_rust_sdk::providers::git::GitCommandProvider;
use log::debug;
use tempfile::TempDir;

use super::init::init_access_tokens;
use super::nix_str_safe;
use crate::config::Config;

#[async_trait]
pub trait FloxCompletionExt
where
    Self: Sized,
{
    /// Create a [Self] ([Flox]) instance in the constrained
    /// context of the [bpaf] completion engine
    fn completion_instance() -> Result<Self>;

    /// Complete an installable from what was already parsed,
    /// informed by applicable flakerefs and prefixes
    async fn complete_installable(
        &self,
        installable_str: &str,
        default_flakerefs: &[&str],
        default_attr_prefixes: &[(&str, bool)],
    ) -> Result<Vec<String>>;
}

#[async_trait]
impl FloxCompletionExt for Flox {
    fn completion_instance() -> Result<Flox> {
        let config = Config::parse()
            .map_err(|e| debug!("Failed to load config: {e}"))
            .unwrap();


        let process_dir = config.flox.cache_dir.join("process");
        match std::fs::create_dir_all(&process_dir) {
            Ok(_) => {},
            Err(e) => {
                bail!("Failed to create process dir: {e}");
            },
        };

        let temp_dir = match TempDir::new_in(process_dir) {
            Ok(x) => x,
            Err(e) => {
                bail!("Failed to create temp_dir: {e}");
            },
        };

        let access_tokens = init_access_tokens(&config.nix.access_tokens)
            .map_err(|e| debug!("Failed to initialize access tokens: {e}"))
            .unwrap_or_default();

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        Ok(Flox {
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            temp_dir: temp_dir.into_path(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            netrc_file,
            access_tokens,
            uuid: uuid::Uuid::nil(),
            floxhub_token: config.flox.floxhub_token,
            floxhub: Floxhub::new(DEFAULT_FLOXHUB_URL.clone()),
        })
    }

    async fn complete_installable(
        &self,
        installable_str: &str,
        default_flakerefs: &[&str],
        default_attr_prefixes: &[(&str, bool)],
    ) -> Result<Vec<String>> {
        let mut flox_installables: Vec<FloxInstallable> = vec![];

        if installable_str != "." {
            let trimmed = installable_str.trim_end_matches(|c| c == '.' || c == '#');

            if let Ok(flox_installable) = trimmed.parse() {
                flox_installables.push(flox_installable);
            }

            match trimmed.rsplit_once(|c| c == '.' || c == '#') {
                Some((s, _)) if s != trimmed => flox_installables.push(s.parse()?),
                None => flox_installables.push("".parse()?),
                Some(_) => {},
            };
        } else {
            flox_installables.push(FloxInstallable {
                source: Some(".".to_string()),
                attr_path: vec![],
            });
        };

        let matches = self
            .resolve_matches::<_, GitCommandProvider>(
                flox_installables.as_slice(),
                default_flakerefs,
                default_attr_prefixes,
                true,
                None,
            )
            .await?;

        let mut prefixes_with: HashMap<String, HashSet<String>> = HashMap::new();
        let mut flakerefs_with: HashMap<String, HashSet<String>> = HashMap::new();

        for m in &matches {
            let k1 = m.key.get(0).expect("match is missing key");

            flakerefs_with
                .entry(k1.clone())
                .or_insert_with(HashSet::new)
                .insert(m.flakeref.clone());

            prefixes_with
                .entry(k1.clone())
                .or_insert_with(HashSet::new)
                .insert(m.prefix.clone());
        }

        let mut completions: Vec<String> = matches
            .iter()
            .flat_map(|m| {
                let nix_safe_key = m
                    .key
                    .iter()
                    .map(|s| nix_str_safe(s.as_str()))
                    .collect::<Vec<_>>()
                    .join(".");

                let mut t = vec![format!(
                    "{}#{}.{}",
                    m.flakeref,
                    nix_str_safe(&m.prefix),
                    nix_safe_key
                )];

                let k1 = m.key.get(0).expect("match is missing key");
                let flakerefs = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);
                let prefixes = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);

                if let (true, Some(system)) = (m.explicit_system, &m.system) {
                    t.push(format!(
                        "{}#{}.{}.{}",
                        m.flakeref,
                        nix_str_safe(&m.prefix),
                        nix_str_safe(system),
                        nix_safe_key
                    ));

                    if flakerefs <= 1 {
                        t.push(format!(
                            "{}.{}.{}",
                            nix_str_safe(&m.prefix),
                            nix_str_safe(system),
                            nix_safe_key
                        ));
                    }
                }

                if flakerefs <= 1 && prefixes <= 1 {
                    t.push(nix_safe_key.clone());
                }

                if prefixes <= 1 {
                    t.push(format!("{}#{}", m.flakeref, nix_safe_key));
                }

                if flakerefs <= 1 {
                    t.push(format!("{}.{}", nix_str_safe(&m.prefix), nix_safe_key));
                }

                t
            })
            .filter(|c| c.starts_with(installable_str))
            .collect();

        completions.sort();
        completions.dedup();

        Ok(completions)
    }
}
