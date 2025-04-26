use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use flox_rust_sdk::flox::{EnvironmentName, EnvironmentOwner, EnvironmentRef, Flox};
use flox_rust_sdk::models::env_registry::garbage_collect;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::raw::PackageToInstall;
use flox_rust_sdk::models::search::PackageDetails;
use flox_rust_sdk::providers::catalog::{ClientTrait, SearchTerm, VersionsError};
use indoc::formatdoc;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, schemars, tool};

use crate::commands::envs::{get_inactive_environments, get_registered_environments};
use crate::commands::list::List;
use crate::commands::search::DEFAULT_SEARCH_LIMIT;
use crate::commands::show::render_show_catalog;
use crate::commands::{
    DisplayEnvironments,
    EnvironmentSelect,
    activated_environments,
    environment_description,
};
use crate::utils::search::DisplaySearchResults;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub enum McpEnvironmentSelect {
    Local {
        #[schemars(description = "path to a local environment")]
        path: PathBuf,
    },
    Remote {
        #[schemars(description = "path to a local environment")]
        owner: String,
        #[schemars(description = "path to a local environment")]
        name: String,
    },
}
#[derive(Debug, Clone)]
pub struct Server {
    flox: Arc<Flox>,
    environment: Arc<Mutex<Option<ConcreteEnvironment>>>,
}

impl Server {
    pub fn new(flox: Arc<Flox>) -> Self {
        Server {
            flox,
            environment: Arc::new(Mutex::new(None)),
        }
    }
}

#[tool(tool_box)]
impl Server {
    #[tool(description = "Search for a package on flox hub")]
    async fn search(
        &self,
        #[tool(param)]
        #[schemars(description = "pacakge name serach term, one word")]
        search_term: String,
    ) -> String {
        let parsed_search_term = match SearchTerm::from_arg(&search_term) {
            SearchTerm::Clean(term) => term,
            SearchTerm::VersionStripped(term) => term,
        };

        let results = self
            .flox
            .catalog_client
            .search_with_spinner(
                parsed_search_term,
                self.flox.system.clone(),
                DEFAULT_SEARCH_LIMIT,
            )
            .await;

        let results = match results {
            Ok(results) => results,
            Err(err) => {
                return formatdoc! {"
                    Search failed.
                    Error:
                    {err}
                "};
            },
        };

        if results.results.is_empty() {
            return "No Results".to_string();
        }

        let results = match DisplaySearchResults::from_search_results(&search_term, results) {
            Ok(results) => results,
            Err(err) => return formatdoc! {"{err}"},
        };

        format!("{results}")
    }

    #[tool(description = "Show more information about a specific package")]
    async fn show(
        &self,
        #[tool(param)]
        #[schemars(description = "pacakge name serach term, one word")]
        search_term: String,
    ) -> Result<rmcp::model::CallToolResult, rmcp::Error> {
        tracing::debug!("using catalog client for show");

        let results = match self
            .flox
            .catalog_client
            .package_versions(&search_term)
            .await
        {
            Ok(results) => results,
            // Below, results.is_empty() is used to mean the search_term
            // didn't match a package.
            // So translate 404 into an empty vec![].
            // Once we drop the pkgdb code path, we can clean this up.
            Err(VersionsError::NotFound) => PackageDetails {
                results: vec![],
                count: None::<u64>,
            },
            Err(e) => Err(rmcp::Error::internal_error(e.to_string(), None))?,
        };
        if results.results.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "no packages matched this pkg-path: '{search_term}'"
            ))]));
        }

        let expected_systems = [
            "aarch64-darwin",
            "aarch64-linux",
            "x86_64-darwin",
            "x86_64-linux",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<HashSet<_>>();
        let rendered = render_show_catalog(&results.results, &expected_systems)
            .map_err(|e| rmcp::Error::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(rendered)]))
    }

    #[tool(description = "list all flox environments on the current machine")]
    async fn list_environments(&self) -> String {
        let active = activated_environments();

        let Ok(env_registry) = garbage_collect(&self.flox) else {
            return "Could not list registered envs".to_string();
        };
        let registered = get_registered_environments(&env_registry);

        let Ok(inactive) = get_inactive_environments(registered, active.iter()) else {
            return "Could not list inactive envs".to_string();
        };

        if active.iter().next().is_none() && inactive.is_empty() {
            return "No environments known to Flox".to_string();
        }

        let mut rendered = String::new();

        if active.iter().next().is_some() {
            writeln!(&mut rendered, "Active environments:");
            let envs =
                indent::indent_all_by(2, DisplayEnvironments::new(active.iter(), true).to_string());
            writeln!(&mut rendered, "{envs}");
        }

        if !inactive.is_empty() {
            writeln!(&mut rendered, "Inactive environments:");
            let envs = indent::indent_all_by(
                2,
                DisplayEnvironments::new(inactive.iter(), false).to_string(),
            );
            writeln!(&mut rendered, "{envs}");
        }

        rendered
    }

    #[tool(description = "report which environemnt to operate on")]
    async fn get_current_environment(&self) -> String {
        match self.environment.lock().unwrap().as_ref() {
            None => "No environment loaded".to_string(),
            Some(ConcreteEnvironment::Path(env)) => {
                let name = env.name();
                let path = env.parent_path();
                format!("Path environment \"{name}\" at {path:?}")
            },
            Some(ConcreteEnvironment::Managed(env)) => {
                let name = env.name();
                let path = env.parent_path();
                format!("Path environment \"{name}\" at {path:?}")
            },
            Some(ConcreteEnvironment::Remote(env)) => {
                let owner = env.owner();
                let name = env.name();

                format!("Remote environment \"{owner}/{name}\"")
            },
        }
    }

    #[tool(description = "load an environement to modify")]
    async fn set_current_environment(&self, #[tool(param)] path: PathBuf) -> String {
        let select = EnvironmentSelect::Dir(path);

        let env = match select.to_concrete_environment(&self.flox) {
            Ok(env) => env,
            Err(err) => return format!("Could not load environemnt: {err}"),
        };

        let description = environment_description(&env).unwrap();

        self.environment.lock().unwrap().insert(env);

        format!("loaded environment {description}")
    }

    #[tool(description = "install packages to the current environment")]
    async fn install(
        &self,
        #[tool(param)]
        #[schemars(description = "packages to install to the current environment")]
        packages: Vec<String>,
    ) -> String {
        let mut guard = self.environment.lock().unwrap();
        let Some(environment) = guard.as_mut() else {
            return "No environment loaded".to_string();
        };

        let handle = tokio::runtime::Handle::current().enter();

        let to_install = match packages
            .iter()
            .map(|p| PackageToInstall::parse(&self.flox.system, p))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(to_install) => to_install,
            Err(err) => return format!("failed to parse package {err}"),
        };

        let result =
            match tokio::task::block_in_place(|| environment.install(&to_install, &self.flox)) {
                Ok(result) => result,
                Err(err) => return format!("Failed to install: {err:?}"),
            };

        format!("install done")

        // let flox = self.flox.clone();
        // let self_env = self.environment.clone();
        // let result = match tokio::task::spawn_blocking(move || {
        //     let mut guard = self_env.lock().unwrap();

        //     let Some(environment) = guard.as_mut() else {
        //         todo!()
        //         // return "No environment loaded".to_string();
        //     };

        //     let to_install = match packages
        //         .iter()
        //         .map(|p| PackageToInstall::parse(&flox.system, p))
        //         .collect::<Result<Vec<_>, _>>()
        //     {
        //         Ok(to_install) => to_install,
        //         Err(err) => todo!(), //  return format!("failed to parse package {err}"),
        //     };
        //     environment.install(&to_install, &flox)
        // })
        // .await
        // {
        //     Ok(result) => result,
        //     Err(err) => return format!("Failed to install: {err:?}"),
        // };
    }

    #[tool(
        description = "report which packages are installed in the current environment with all detail"
    )]
    async fn list_packages(&self) -> String {
        let mut guard = self.environment.lock().unwrap();
        let Some(environment) = guard.as_mut() else {
            return "No environment loaded".to_string();
        };

        let lockfile: Lockfile = match environment.lockfile(&self.flox) {
            Ok(lf) => lf.into(),
            Err(err) => return format!("Could not get lockfile: {err}"),
        };

        let system = &self.flox.system;
        let packages = match lockfile.list_packages(system) {
            Ok(packages) => packages,
            Err(err) => return format!("Could not list packages: {err}"),
        };

        let mut rendered = Vec::new();
        List::print_detail(&mut rendered, &packages, None);

        let rendered = String::from_utf8(rendered).unwrap();

        rendered
    }

    // fn sum(&self, #[tool(aggr)] SumRequest { a, b }: SumRequest) -> String {
    //     (a + b).to_string()
    // }

    // #[tool(description = "Calculate the sum of two numbers")]
    // fn sub(
    //     &self,
    //     #[tool(param)]
    //     #[schemars(description = "the left hand side number")]
    //     a: i32,
    //     #[tool(param)]
    //     #[schemars(description = "the left hand side number")]
    //     b: i32,
    // ) -> String {
    //     (a - b).to_string()
    // }
}

#[tool(tool_box)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A toolbox for flox".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
