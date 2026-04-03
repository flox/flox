use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Result, bail};
use clap::Args;
use serde::Deserialize;

use super::fix_paths::prepend_dirs_to_pathlike_var;
use super::{join_dir_list, separate_dir_list};

/// Nix-substituted tool paths read from tool-paths.json in the interpreter package.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolPaths {
    pub ld_floxlib: String,
    pub ldconfig: String,
    pub find: String,
}

impl ToolPaths {
    /// Read tool paths from the interpreter package directory.
    pub fn from_interpreter(interpreter_path: &Path) -> Result<Self> {
        let path = interpreter_path.join("tool-paths.json");
        let contents = fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", path.display()))?;
        let paths: ToolPaths = serde_json::from_str(&contents)?;
        Ok(paths)
    }

    /// Default paths for when tool-paths.json is not available (e.g. tests).
    pub fn defaults() -> Self {
        Self {
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find: "".to_string(),
        }
    }
}

/// Parameters for computing profile environment variables.
/// This is the pure data struct used by both the CLI subcommand and in-process calls.
#[derive(Debug, Clone)]
pub struct ProfileEnvConfig {
    pub mode: String,
    pub flox_env: PathBuf,
    pub env_dirs: String,
    pub ld_floxlib: String,
    pub ldconfig: String,
    pub find_bin: String,
    pub env_project: Option<PathBuf>,
}

/// Compute all profile.d environment variable changes and return them as a HashMap.
/// This is the core logic shared between the CLI subcommand and in-process calls from start.rs.
pub fn compute_profile_env(config: &ProfileEnvConfig) -> Result<HashMap<String, String>> {
    let mut exports: Vec<(&str, String)> = Vec::new();
    let env_dirs = separate_dir_list(&config.env_dirs);
    let mode = config.mode.as_str();

    match mode {
        "run" => {
            setup_run_mode_paths(&config.flox_env, &mut exports);
        },
        "dev" | "build" => {
            setup_run_mode_paths(&config.flox_env, &mut exports);
            setup_dev_mode_paths(&config.flox_env, &config.ld_floxlib, &mut exports, &env_dirs);
            setup_languages(&config.flox_env, &mut exports);
            setup_cuda(&config.ldconfig, &mut exports)?;
            setup_python(&config.flox_env, config.env_project.as_deref(), &mut exports, mode, &env_dirs)?;
            setup_cmake(&config.flox_env, &mut exports, mode, &env_dirs);
        },
        other => bail!("invalid mode: {other}"),
    }

    Ok(exports
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect())
}

/// Parse an envrc file and return the environment variable assignments.
/// Handles `export KEY="VALUE"` and `export KEY="${KEY:-default}"` patterns.
pub fn parse_envrc(envrc_path: &Path) -> Result<HashMap<String, String>> {
    let contents = match fs::read_to_string(envrc_path) {
        Ok(c) => c,
        Err(_) => return Ok(HashMap::new()),
    };

    let mut vars = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Match: export KEY="VALUE"
        if let Some(rest) = line.strip_prefix("export ") {
            if let Some(eq_pos) = rest.find('=') {
                let key = rest[..eq_pos].trim();
                let raw_value = rest[eq_pos + 1..].trim();

                // Strip surrounding quotes
                let value = if (raw_value.starts_with('"') && raw_value.ends_with('"'))
                    || (raw_value.starts_with('\'') && raw_value.ends_with('\''))
                {
                    &raw_value[1..raw_value.len() - 1]
                } else {
                    raw_value
                };

                // Evaluate ${VAR:-default} patterns
                let evaluated = evaluate_bash_defaults(value);
                vars.insert(key.to_string(), evaluated);
            }
        }
    }

    Ok(vars)
}

/// Evaluate bash `${VAR:-default}` and `${VAR}` patterns in a string value.
/// Only handles the simple `${VAR:-default}` form used in envrc files.
fn evaluate_bash_defaults(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            // Read the variable name and optional default
            let mut inner = String::new();
            let mut depth = 1;
            while let Some(c) = chars.next() {
                if c == '{' {
                    depth += 1;
                    inner.push(c);
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    inner.push(c);
                } else {
                    inner.push(c);
                }
            }

            // Parse VAR:-default
            if let Some(sep_pos) = inner.find(":-") {
                let var_name = &inner[..sep_pos];
                let default_val = &inner[sep_pos + 2..];
                // Check if the env var is set and non-empty
                match std::env::var(var_name) {
                    Ok(val) if !val.is_empty() => result.push_str(&val),
                    _ => {
                        // Default might contain nested ${} references
                        let evaluated_default = evaluate_bash_defaults(default_val);
                        result.push_str(&evaluated_default);
                    },
                }
            } else {
                // Simple ${VAR} reference
                if let Ok(val) = std::env::var(&inner) {
                    result.push_str(&val);
                }
                // If unset, expand to empty string
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// CLI subcommand that outputs shell-sourceable export statements.
/// This is a thin wrapper around compute_profile_env().
#[derive(Debug, Args)]
pub struct SetupEnvArgs {
    #[arg(long)]
    pub mode: String,
    #[arg(long)]
    pub flox_env: PathBuf,
    #[arg(long, default_value = "")]
    pub env_dirs: String,
    #[arg(long, default_value = "bash")]
    pub shell: String,
    #[arg(long, default_value = "__LINUX_ONLY__")]
    pub ld_floxlib: String,
    #[arg(long, default_value = "__LINUX_ONLY__")]
    pub ldconfig: String,
    #[arg(long, default_value = "")]
    pub find_bin: String,
    #[arg(long)]
    pub env_project: Option<PathBuf>,
    #[arg(long)]
    pub dump_env_start: Option<PathBuf>,
}

impl SetupEnvArgs {
    pub fn handle(&self) -> Result<()> {
        if let Some(ref path) = self.dump_env_start {
            let env_map: HashMap<String, String> = std::env::vars().collect();
            let file = fs::File::create(path)?;
            let mut writer = std::io::BufWriter::new(file);
            serde_json::to_writer(&mut writer, &env_map)?;
            std::io::Write::write_all(&mut writer, b"\n")?;
        }

        let config = ProfileEnvConfig {
            mode: self.mode.clone(),
            flox_env: self.flox_env.clone(),
            env_dirs: self.env_dirs.clone(),
            ld_floxlib: self.ld_floxlib.clone(),
            ldconfig: self.ldconfig.clone(),
            find_bin: self.find_bin.clone(),
            env_project: self.env_project.clone(),
        };

        let exports = compute_profile_env(&config)?;
        let sourceable = format_exports_shell(&self.shell, &exports);
        let mut stdout = std::io::stdout();
        stdout.write_all(sourceable.as_bytes())?;
        Ok(())
    }
}

/// Format a HashMap of exports as shell-sourceable statements.
pub fn format_exports_shell(shell: &str, exports: &HashMap<String, String>) -> String {
    let mut result = String::new();
    // Sort for deterministic output
    let mut keys: Vec<&String> = exports.keys().collect();
    keys.sort();
    for key in keys {
        let value = &exports[key];
        match shell {
            "bash" | "zsh" => {
                result.push_str(&format!("export {key}=\"{value}\";\n"));
            },
            "fish" => {
                result.push_str(&format!("set -gx {key} \"{value}\";\n"));
            },
            "tcsh" => {
                result.push_str(&format!("setenv {key} \"{value}\";\n"));
            },
            _ => {
                result.push_str(&format!("export {key}=\"{value}\";\n"));
            },
        }
    }
    result
}

// ── Profile.d computation functions (free functions for reuse) ──────────

fn setup_run_mode_paths(flox_env: &Path, exports: &mut Vec<(&str, String)>) {
    let flox_env = flox_env.to_string_lossy();

    let infopath = std::env::var("INFOPATH").unwrap_or_default();
    exports.push((
        "INFOPATH",
        if infopath.is_empty() {
            format!("{flox_env}/share/info")
        } else {
            format!("{flox_env}/share/info:{infopath}")
        },
    ));

    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    exports.push((
        "XDG_DATA_DIRS",
        if xdg.is_empty() {
            format!("{flox_env}/share")
        } else {
            format!("{flox_env}/share:{xdg}")
        },
    ));
}

fn setup_dev_mode_paths(
    flox_env: &Path,
    ld_floxlib: &str,
    exports: &mut Vec<(&str, String)>,
    env_dirs: &[PathBuf],
) {
    let flox_env_str = flox_env.to_string_lossy();

    let cpath = std::env::var("CPATH").unwrap_or_default();
    exports.push((
        "CPATH",
        prepend_path(&format!("{flox_env_str}/include"), &cpath),
    ));

    let library_path = std::env::var("LIBRARY_PATH").unwrap_or_default();
    exports.push((
        "LIBRARY_PATH",
        prepend_path(&format!("{flox_env_str}/lib"), &library_path),
    ));

    let pkg_config = std::env::var("PKG_CONFIG_PATH").unwrap_or_default();
    let new_pkg = format!(
        "{flox_env_str}/lib/pkgconfig:{flox_env_str}/share/pkgconfig{}",
        if pkg_config.is_empty() {
            String::new()
        } else {
            format!(":{pkg_config}")
        }
    );
    exports.push(("PKG_CONFIG_PATH", new_pkg));

    let aclocal = std::env::var("ACLOCAL_PATH").unwrap_or_default();
    exports.push((
        "ACLOCAL_PATH",
        prepend_path(&format!("{flox_env_str}/share/aclocal"), &aclocal),
    ));

    if !env_dirs.is_empty() {
        setup_platform_libs(ld_floxlib, exports, env_dirs);
    }
}

fn setup_platform_libs(ld_floxlib: &str, exports: &mut Vec<(&str, String)>, env_dirs: &[PathBuf]) {
    if cfg!(target_os = "linux") {
        let ld_floxlib_val = std::env::var("LD_FLOXLIB").unwrap_or_else(|_| ld_floxlib.to_string());
        let noset = std::env::var("FLOX_NOSET_LD_AUDIT").unwrap_or_default();

        if noset.is_empty()
            && ld_floxlib_val != "__LINUX_ONLY__"
            && Path::new(&ld_floxlib_val).exists()
        {
            exports.push(("LD_AUDIT", ld_floxlib_val));
            exports.push((
                "GLIBC_TUNABLES",
                "glibc.rtld.optional_static_tls=25000".to_string(),
            ));
        }
    } else if cfg!(target_os = "macos") {
        let noset = std::env::var("FLOX_NOSET_DYLD_FALLBACK_LIBRARY_PATH").unwrap_or_default();
        if noset.is_empty() {
            let lib_dirs: Vec<String> = env_dirs
                .iter()
                .map(|d| format!("{}/lib", d.to_string_lossy()))
                .collect();
            let existing = std::env::var("DYLD_FALLBACK_LIBRARY_PATH")
                .unwrap_or_else(|_| "/usr/local/lib:/usr/lib".to_string());
            let new_val = if lib_dirs.is_empty() {
                existing
            } else {
                format!("{}:{existing}", lib_dirs.join(":"))
            };
            exports.push(("DYLD_FALLBACK_LIBRARY_PATH", new_val));
        }
    }
}

fn setup_languages(flox_env: &Path, exports: &mut Vec<(&str, String)>) {
    if flox_env.join("rustc-std-workspace-std").is_dir() {
        exports.push(("RUST_SRC_PATH", flox_env.to_string_lossy().into_owned()));
    }

    let jupyter_dir = flox_env.join("share/jupyter");
    if jupyter_dir.is_dir() {
        let existing = std::env::var("JUPYTER_PATH").unwrap_or_default();
        let new_val = if existing.is_empty() {
            jupyter_dir.to_string_lossy().into_owned()
        } else {
            format!("{}:{existing}", jupyter_dir.to_string_lossy())
        };
        exports.push(("JUPYTER_PATH", new_val));
    }

    let java_bin = flox_env.join("bin/java");
    if java_bin.exists() && is_executable(&java_bin) {
        exports.push(("JAVA_HOME", flox_env.to_string_lossy().into_owned()));
    }
}

fn setup_cuda(ldconfig: &str, exports: &mut Vec<(&str, String)>) -> Result<()> {
    let cuda_detection = std::env::var("_FLOX_ENV_CUDA_DETECTION").unwrap_or_default();
    if cuda_detection != "1" {
        return Ok(());
    }

    if !cfg!(target_os = "linux") {
        return Ok(());
    }

    if ldconfig == "__LINUX_ONLY__"
        || !Path::new(ldconfig).exists()
        || !is_executable(Path::new(ldconfig))
    {
        return Ok(());
    }

    if !has_nvidia_device()? {
        return Ok(());
    }

    let mut system_libs = find_cuda_libs_ldconfig(ldconfig)?;
    if system_libs.is_empty() {
        system_libs = find_cuda_libs_nixos()?;
    }

    if system_libs.is_empty() {
        return Ok(());
    }

    let existing = std::env::var("LD_FLOXLIB_FILES_PATH").unwrap_or_default();
    let new_val = if existing.is_empty() {
        system_libs.join(":")
    } else {
        format!("{existing}:{}", system_libs.join(":"))
    };
    exports.push(("LD_FLOXLIB_FILES_PATH", new_val));
    Ok(())
}

fn find_cuda_libs_ldconfig(ldconfig: &str) -> Result<Vec<String>> {
    let output = ProcessCommand::new(ldconfig)
        .args(["--print-cache", "-C", "/etc/ld.so.cache"])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => return Ok(vec![]),
    };

    if !output.status.success() {
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut libs = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(lib_name) = trimmed.split_whitespace().next() {
            if is_cuda_lib(lib_name) {
                if let Some(path) = trimmed.rsplit(" => ").next() {
                    libs.push(path.to_string());
                }
            }
        }
    }
    Ok(libs)
}

fn setup_python(
    flox_env: &Path,
    env_project: Option<&Path>,
    exports: &mut Vec<(&str, String)>,
    mode: &str,
    env_dirs: &[PathBuf],
) -> Result<()> {
    let python_bin = flox_env.join("bin/python3");
    if !python_bin.exists() || !is_executable(&python_bin) {
        return Ok(());
    }

    let python_version = detect_python_version(flox_env)?;
    if let Some(version) = python_version {
        let suffix = format!("lib/python{version}/site-packages");

        if mode == "set" || mode == "build" {
            exports.push((
                "PYTHONPATH",
                format!("{}/{suffix}", flox_env.to_string_lossy()),
            ));
        } else {
            let existing = std::env::var("PYTHONPATH").unwrap_or_default();
            let existing_dirs = separate_dir_list(&existing);
            let new_dirs = prepend_dirs_to_pathlike_var(env_dirs, &[&suffix], &existing_dirs);
            exports.push(("PYTHONPATH", join_dir_list(new_dirs)));
        }
    }

    let pip_bin = flox_env.join("bin/pip3");
    if pip_bin.exists() && is_executable(&pip_bin) {
        if let Some(env_project) = env_project {
            let pip_config = env_project.join(".flox/pip.ini");
            if let Some(parent) = pip_config.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&pip_config, "  [global]\n  require-virtualenv = true\n");
            exports.push((
                "PIP_CONFIG_FILE",
                pip_config.to_string_lossy().into_owned(),
            ));
        }
    }

    Ok(())
}

fn setup_cmake(
    flox_env: &Path,
    exports: &mut Vec<(&str, String)>,
    mode: &str,
    env_dirs: &[PathBuf],
) {
    let cmake_bin = flox_env.join("bin/cmake");
    if !cmake_bin.exists() || !is_executable(&cmake_bin) {
        return;
    }

    if mode == "set" || mode == "build" {
        exports.push((
            "CMAKE_PREFIX_PATH",
            flox_env.to_string_lossy().into_owned(),
        ));
    } else {
        let existing = std::env::var("CMAKE_PREFIX_PATH").unwrap_or_default();
        let existing_dirs = separate_dir_list(&existing);
        let new_dirs = prepend_dirs_to_pathlike_var(env_dirs, &[] as &[&str], &existing_dirs);
        exports.push(("CMAKE_PREFIX_PATH", join_dir_list(new_dirs)));
    }
}

// ── Helper functions ────────────────────────────────────────────────────

fn prepend_path(new: &str, existing: &str) -> String {
    if existing.is_empty() {
        new.to_string()
    } else {
        format!("{new}:{existing}")
    }
}

fn detect_python_version(flox_env: &Path) -> Result<Option<String>> {
    let lib_dir = flox_env.join("lib");
    if !lib_dir.is_dir() {
        return Ok(None);
    }

    let entries = match fs::read_dir(&lib_dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(version) = name_str.strip_prefix("python") {
            if version.contains('.')
                && version.chars().next().is_some_and(|c| c.is_ascii_digit())
                && entry.path().join("site-packages").is_dir()
            {
                return Ok(Some(version.to_string()));
            }
        }
    }

    Ok(None)
}

fn has_nvidia_device() -> Result<bool> {
    let dev_path = Path::new("/dev");
    if !dev_path.is_dir() {
        return Ok(false);
    }

    let entries = match fs::read_dir(dev_path) {
        Ok(e) => e,
        Err(_) => return Ok(false),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_lower = name.to_string_lossy().to_lowercase();
        if name_lower.starts_with("nvidia") || name_lower == "dxg" {
            return Ok(true);
        }
    }

    Ok(false)
}

fn find_cuda_libs_nixos() -> Result<Vec<String>> {
    let opengl_dir = Path::new("/run/opengl-driver");
    if !opengl_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut libs = Vec::new();
    walk_for_cuda_libs(opengl_dir, &mut libs)?;
    Ok(libs)
}

fn walk_for_cuda_libs(dir: &Path, libs: &mut Vec<String>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let metadata = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            walk_for_cuda_libs(&path, libs)?;
        } else if metadata.is_file() {
            if let Some(name) = path.file_name() {
                if is_cuda_lib(&name.to_string_lossy()) {
                    libs.push(path.to_string_lossy().into_owned());
                }
            }
        }
    }

    Ok(())
}

fn is_cuda_lib(name: &str) -> bool {
    let starts_ok = name.starts_with("libcuda")
        || name.starts_with("libnv")
        || name.starts_with("libdxcore");
    starts_ok && name.contains(".so")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::metadata(path) {
            Ok(m) => m.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn test_config(mode: &str, flox_env: &str, env_dirs: &str) -> ProfileEnvConfig {
        ProfileEnvConfig {
            mode: mode.to_string(),
            flox_env: PathBuf::from(flox_env),
            env_dirs: env_dirs.to_string(),
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find_bin: "".to_string(),
            env_project: None,
        }
    }

    #[test]
    fn run_mode_sets_infopath_and_xdg() {
        let config = test_config("run", "/env1", "");
        let exports = compute_profile_env(&config).unwrap();
        assert!(exports.get("INFOPATH").unwrap().contains("/env1/share/info"));
        assert!(exports.get("XDG_DATA_DIRS").unwrap().contains("/env1/share"));
    }

    #[test]
    fn dev_mode_sets_dev_paths() {
        let config = test_config("dev", "/env1", "/env1");
        let exports = compute_profile_env(&config).unwrap();
        assert!(exports.get("CPATH").unwrap().contains("/env1/include"));
        assert!(exports.get("LIBRARY_PATH").unwrap().contains("/env1/lib"));
        assert!(exports
            .get("PKG_CONFIG_PATH")
            .unwrap()
            .contains("/env1/lib/pkgconfig"));
    }

    #[test]
    fn detect_python_version_no_lib_dir() {
        let result = detect_python_version(Path::new("/nonexistent")).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn format_exports_fish() {
        let mut exports = HashMap::new();
        exports.insert("FOO".to_string(), "bar".to_string());
        let result = format_exports_shell("fish", &exports);
        assert_eq!(result, "set -gx FOO \"bar\";\n");
    }

    #[test]
    fn format_exports_tcsh() {
        let mut exports = HashMap::new();
        exports.insert("FOO".to_string(), "bar".to_string());
        let result = format_exports_shell("tcsh", &exports);
        assert_eq!(result, "setenv FOO \"bar\";\n");
    }

    #[test]
    fn prepend_path_empty() {
        assert_eq!(prepend_path("/new", ""), "/new");
    }

    #[test]
    fn prepend_path_existing() {
        assert_eq!(prepend_path("/new", "/old"), "/new:/old");
    }

    #[test]
    fn lines_have_trailing_semicolons() {
        let shells = ["bash", "zsh", "fish", "tcsh"];
        for shell in shells {
            let config = test_config("run", "/env1", "");
            let exports = compute_profile_env(&config).unwrap();
            let output = format_exports_shell(shell, &exports);
            for line in output.lines() {
                if !line.is_empty() {
                    assert!(line.ends_with(';'), "line missing semicolon: {line}");
                }
            }
        }
    }

    #[test]
    fn evaluate_simple_value() {
        assert_eq!(evaluate_bash_defaults("hello"), "hello");
    }

    #[test]
    fn evaluate_default_when_unset() {
        // Use a var name unlikely to be set
        let result = evaluate_bash_defaults("${_FLOX_TEST_UNLIKELY_VAR_XYZ:-fallback}");
        assert_eq!(result, "fallback");
    }

    #[test]
    fn evaluate_nested_default() {
        let result = evaluate_bash_defaults(
            "${_FLOX_TEST_UNLIKELY_A:-${_FLOX_TEST_UNLIKELY_B:-deep}}",
        );
        assert_eq!(result, "deep");
    }

    #[test]
    fn parse_envrc_basic() {
        let dir = tempfile::tempdir().unwrap();
        let envrc = dir.path().join("envrc");
        fs::write(
            &envrc,
            "# comment\nexport FOO=\"bar\"\nexport BAZ=\"qux\"\n",
        )
        .unwrap();
        let vars = parse_envrc(&envrc).unwrap();
        assert_eq!(vars.get("FOO").unwrap(), "bar");
        assert_eq!(vars.get("BAZ").unwrap(), "qux");
    }

    #[test]
    fn parse_envrc_with_default() {
        let dir = tempfile::tempdir().unwrap();
        let envrc = dir.path().join("envrc");
        fs::write(
            &envrc,
            "export MY_VAR=\"${_FLOX_TEST_NONEXISTENT:-/default/path}\"\n",
        )
        .unwrap();
        let vars = parse_envrc(&envrc).unwrap();
        assert_eq!(vars.get("MY_VAR").unwrap(), "/default/path");
    }

    #[test]
    fn parse_envrc_missing_file() {
        let vars = parse_envrc(Path::new("/nonexistent/envrc")).unwrap();
        assert!(vars.is_empty());
    }
}
