use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Result, bail};
use clap::Args;

use super::fix_paths::prepend_dirs_to_pathlike_var;
use super::{join_dir_list, separate_dir_list};

/// Compute all profile.d environment variable changes in a single process,
/// replacing the need to source multiple shell scripts.
///
/// This replaces: 0100_common-run-mode-paths.sh, 0101_common-dev-mode-additional-paths.sh,
/// 0500_languages.sh, 0800_cuda.sh, _setup_python, and _setup_cmake.
///
/// Outputs shell-sourceable export statements.
#[derive(Debug, Args)]
pub struct SetupEnvArgs {
    /// Activation mode: dev, run, or build.
    #[arg(long)]
    pub mode: String,

    /// The FLOX_ENV path (symlink to environment store path).
    #[arg(long)]
    pub flox_env: PathBuf,

    /// Colon-separated FLOX_ENV_DIRS.
    #[arg(long, default_value = "")]
    pub env_dirs: String,

    /// Which shell syntax to emit (bash, zsh, fish, tcsh).
    #[arg(long, default_value = "bash")]
    pub shell: String,

    /// Path to ld-floxlib.so (Linux only, "__LINUX_ONLY__" on Darwin).
    #[arg(long, default_value = "__LINUX_ONLY__")]
    pub ld_floxlib: String,

    /// Path to ldconfig binary (for CUDA detection).
    #[arg(long, default_value = "__LINUX_ONLY__")]
    pub ldconfig: String,

    /// Path to find binary (for CUDA detection).
    #[arg(long, default_value = "")]
    pub find_bin: String,

    /// Path to FLOX_ENV_PROJECT (optional, for pip.ini creation).
    #[arg(long)]
    pub env_project: Option<PathBuf>,
}

impl SetupEnvArgs {
    pub fn handle(&self) -> Result<()> {
        let mut stdout = std::io::stdout();
        self.handle_inner(&mut stdout)
    }

    fn handle_inner(&self, output: &mut impl Write) -> Result<()> {
        let mut exports: Vec<(&str, String)> = Vec::new();

        let env_dirs = separate_dir_list(&self.env_dirs);
        let mode = self.mode.as_str();

        match mode {
            "run" => {
                self.setup_run_mode_paths(&mut exports);
            },
            "dev" | "build" => {
                self.setup_run_mode_paths(&mut exports);
                self.setup_dev_mode_paths(&mut exports, &env_dirs);
                self.setup_languages(&mut exports);
                self.setup_cuda(&mut exports)?;
                self.setup_python(&mut exports, mode, &env_dirs)?;
                self.setup_cmake(&mut exports, mode, &env_dirs);
            },
            other => bail!("invalid mode: {other}"),
        }

        let sourceable = self.format_exports(&exports);
        output.write_all(sourceable.as_bytes())?;
        Ok(())
    }

    /// 0100_common-run-mode-paths.sh equivalent
    fn setup_run_mode_paths(&self, exports: &mut Vec<(&str, String)>) {
        let flox_env = self.flox_env.to_string_lossy();

        let infopath = std::env::var("INFOPATH").unwrap_or_default();
        let new_infopath = if infopath.is_empty() {
            format!("{flox_env}/share/info")
        } else {
            format!("{flox_env}/share/info:{infopath}")
        };
        exports.push(("INFOPATH", new_infopath));

        let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        let new_xdg = if xdg.is_empty() {
            format!("{flox_env}/share")
        } else {
            format!("{flox_env}/share:{xdg}")
        };
        exports.push(("XDG_DATA_DIRS", new_xdg));
    }

    /// 0101_common-dev-mode-additional-paths.sh equivalent
    fn setup_dev_mode_paths(&self, exports: &mut Vec<(&str, String)>, env_dirs: &[PathBuf]) {
        let flox_env = self.flox_env.to_string_lossy();

        // Simple path prepends
        let cpath = std::env::var("CPATH").unwrap_or_default();
        exports.push((
            "CPATH",
            prepend_path(&format!("{flox_env}/include"), &cpath),
        ));

        let library_path = std::env::var("LIBRARY_PATH").unwrap_or_default();
        exports.push((
            "LIBRARY_PATH",
            prepend_path(&format!("{flox_env}/lib"), &library_path),
        ));

        let pkg_config = std::env::var("PKG_CONFIG_PATH").unwrap_or_default();
        let new_pkg = format!(
            "{flox_env}/lib/pkgconfig:{flox_env}/share/pkgconfig{}",
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
            prepend_path(&format!("{flox_env}/share/aclocal"), &aclocal),
        ));

        // Platform-specific library loading
        if !env_dirs.is_empty() {
            self.setup_platform_libs(exports, env_dirs);
        }
    }

    /// Platform-specific library loading setup (LD_AUDIT on Linux, DYLD on macOS)
    fn setup_platform_libs(&self, exports: &mut Vec<(&str, String)>, env_dirs: &[PathBuf]) {
        if cfg!(target_os = "linux") {
            // LD_AUDIT setup
            let ld_floxlib = std::env::var("LD_FLOXLIB")
                .unwrap_or_else(|_| self.ld_floxlib.clone());
            let noset = std::env::var("FLOX_NOSET_LD_AUDIT")
                .unwrap_or_default();

            if noset.is_empty()
                && ld_floxlib != "__LINUX_ONLY__"
                && Path::new(&ld_floxlib).exists()
            {
                exports.push(("LD_AUDIT", ld_floxlib));
                exports.push((
                    "GLIBC_TUNABLES",
                    "glibc.rtld.optional_static_tls=25000".to_string(),
                ));
            }
        } else if cfg!(target_os = "macos") {
            let noset = std::env::var("FLOX_NOSET_DYLD_FALLBACK_LIBRARY_PATH")
                .unwrap_or_default();
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

    /// 0500_languages.sh equivalent
    fn setup_languages(&self, exports: &mut Vec<(&str, String)>) {
        let flox_env = &self.flox_env;

        // Rust: RUST_SRC_PATH
        if flox_env.join("rustc-std-workspace-std").is_dir() {
            exports.push(("RUST_SRC_PATH", flox_env.to_string_lossy().into_owned()));
        }

        // Jupyter: JUPYTER_PATH
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

        // Java: JAVA_HOME
        let java_bin = flox_env.join("bin/java");
        if java_bin.exists() && is_executable(&java_bin) {
            exports.push(("JAVA_HOME", flox_env.to_string_lossy().into_owned()));
        }
    }

    /// 0800_cuda.sh equivalent
    fn setup_cuda(&self, exports: &mut Vec<(&str, String)>) -> Result<()> {
        let cuda_detection = std::env::var("_FLOX_ENV_CUDA_DETECTION")
            .unwrap_or_default();
        if cuda_detection != "1" {
            return Ok(());
        }

        // Only on Linux
        if !cfg!(target_os = "linux") {
            return Ok(());
        }

        let ldconfig = &self.ldconfig;
        if ldconfig == "__LINUX_ONLY__"
            || !Path::new(ldconfig).exists()
            || !is_executable(Path::new(ldconfig))
        {
            return Ok(());
        }

        // Check for nvidia device files
        if !has_nvidia_device(&self.find_bin)? {
            return Ok(());
        }

        // Try system ldconfig cache
        let mut system_libs = self.find_cuda_libs_ldconfig(ldconfig)?;

        // Fallback: NixOS /run/opengl-driver
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

    /// Parse ldconfig --print-cache output to find CUDA libraries
    fn find_cuda_libs_ldconfig(
        &self,
        ldconfig: &str,
    ) -> Result<Vec<String>> {
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
            // ldconfig output format: "libname.so.1 (libc6,...) => /path/to/lib.so.1"
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

    /// Python setup: detect version via filesystem glob, set PYTHONPATH, create pip.ini
    fn setup_python(
        &self,
        exports: &mut Vec<(&str, String)>,
        mode: &str,
        env_dirs: &[PathBuf],
    ) -> Result<()> {
        let python_bin = self.flox_env.join("bin/python3");
        if !python_bin.exists() || !is_executable(&python_bin) {
            return Ok(());
        }

        // Detect Python version from filesystem instead of spawning python3
        let python_version = detect_python_version(&self.flox_env)?;
        if let Some(version) = python_version {
            let suffix = format!("lib/python{version}/site-packages");

            if mode == "set" || mode == "build" {
                exports.push((
                    "PYTHONPATH",
                    format!("{}/{suffix}", self.flox_env.to_string_lossy()),
                ));
            } else {
                // prepend mode
                let existing = std::env::var("PYTHONPATH").unwrap_or_default();
                let existing_dirs = separate_dir_list(&existing);
                let new_dirs =
                    prepend_dirs_to_pathlike_var(env_dirs, &[&suffix], &existing_dirs);
                exports.push(("PYTHONPATH", join_dir_list(new_dirs)));
            }
        }

        // pip.ini creation
        let pip_bin = self.flox_env.join("bin/pip3");
        if pip_bin.exists() && is_executable(&pip_bin) {
            if let Some(ref env_project) = self.env_project {
                let pip_config = env_project.join(".flox/pip.ini");
                if let Some(parent) = pip_config.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(
                    &pip_config,
                    "  [global]\n  require-virtualenv = true\n",
                );
                exports.push((
                    "PIP_CONFIG_FILE",
                    pip_config.to_string_lossy().into_owned(),
                ));
            }
        }

        Ok(())
    }

    /// CMake setup: set CMAKE_PREFIX_PATH
    fn setup_cmake(
        &self,
        exports: &mut Vec<(&str, String)>,
        mode: &str,
        env_dirs: &[PathBuf],
    ) {
        let cmake_bin = self.flox_env.join("bin/cmake");
        if !cmake_bin.exists() || !is_executable(&cmake_bin) {
            return;
        }

        if mode == "set" || mode == "build" {
            exports.push((
                "CMAKE_PREFIX_PATH",
                self.flox_env.to_string_lossy().into_owned(),
            ));
        } else {
            let existing = std::env::var("CMAKE_PREFIX_PATH").unwrap_or_default();
            let existing_dirs = separate_dir_list(&existing);
            let new_dirs =
                prepend_dirs_to_pathlike_var(env_dirs, &[] as &[&str], &existing_dirs);
            exports.push(("CMAKE_PREFIX_PATH", join_dir_list(new_dirs)));
        }
    }

    /// Format exports as shell-sourceable statements
    fn format_exports(&self, exports: &[(&str, String)]) -> String {
        let mut result = String::new();
        for (key, value) in exports {
            match self.shell.as_str() {
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
                    // Default to bash syntax
                    result.push_str(&format!("export {key}=\"{value}\";\n"));
                },
            }
        }
        result
    }
}

/// Simple path prepend helper
fn prepend_path(new: &str, existing: &str) -> String {
    if existing.is_empty() {
        new.to_string()
    } else {
        format!("{new}:{existing}")
    }
}

/// Detect Python version by scanning $FLOX_ENV/lib/python*/site-packages/
/// This avoids spawning a python3 subprocess (~15-25ms savings).
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
            // Verify it looks like a version (e.g., "3.12") and has site-packages
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

/// Check for nvidia device files without spawning find
fn has_nvidia_device(_find_bin: &str) -> Result<bool> {
    // Check /dev directly instead of spawning find
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

/// Find CUDA libraries in NixOS /run/opengl-driver (fallback)
fn find_cuda_libs_nixos() -> Result<Vec<String>> {
    let opengl_dir = Path::new("/run/opengl-driver");
    if !opengl_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut libs = Vec::new();
    walk_for_cuda_libs(opengl_dir, &mut libs)?;
    Ok(libs)
}

/// Recursively walk directory for CUDA library files
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
        // Follow symlinks
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

/// Check if a library name matches the CUDA library pattern.
/// Equivalent to regex: `^lib(cuda|nv|dxcore).*\.so`
fn is_cuda_lib(name: &str) -> bool {
    let starts_ok = name.starts_with("libcuda")
        || name.starts_with("libnv")
        || name.starts_with("libdxcore");
    starts_ok && name.contains(".so")
}

/// Check if a path is executable (cross-platform)
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

    #[test]
    fn run_mode_sets_infopath_and_xdg() {
        let mut buf = vec![];
        let args = SetupEnvArgs {
            mode: "run".to_string(),
            flox_env: PathBuf::from("/env1"),
            env_dirs: "".to_string(),
            shell: "bash".to_string(),
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find_bin: "".to_string(),
            env_project: None,
        };
        args.handle_inner(&mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("INFOPATH=\"/env1/share/info"));
        assert!(output.contains("XDG_DATA_DIRS=\"/env1/share"));
    }

    #[test]
    fn dev_mode_sets_dev_paths() {
        let mut buf = vec![];
        let args = SetupEnvArgs {
            mode: "dev".to_string(),
            flox_env: PathBuf::from("/env1"),
            env_dirs: "/env1".to_string(),
            shell: "bash".to_string(),
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find_bin: "".to_string(),
            env_project: None,
        };
        args.handle_inner(&mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("CPATH=\"/env1/include"));
        assert!(output.contains("LIBRARY_PATH=\"/env1/lib"));
        assert!(output.contains("PKG_CONFIG_PATH=\"/env1/lib/pkgconfig:/env1/share/pkgconfig"));
        assert!(output.contains("ACLOCAL_PATH=\"/env1/share/aclocal"));
    }

    #[test]
    fn detect_python_version_no_lib_dir() {
        let result = detect_python_version(Path::new("/nonexistent")).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn format_exports_fish() {
        let args = SetupEnvArgs {
            mode: "run".to_string(),
            flox_env: PathBuf::from("/env1"),
            env_dirs: "".to_string(),
            shell: "fish".to_string(),
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find_bin: "".to_string(),
            env_project: None,
        };
        let exports = vec![("FOO", "bar".to_string())];
        let result = args.format_exports(&exports);
        assert_eq!(result, "set -gx FOO \"bar\";\n");
    }

    #[test]
    fn format_exports_tcsh() {
        let args = SetupEnvArgs {
            mode: "run".to_string(),
            flox_env: PathBuf::from("/env1"),
            env_dirs: "".to_string(),
            shell: "tcsh".to_string(),
            ld_floxlib: "__LINUX_ONLY__".to_string(),
            ldconfig: "__LINUX_ONLY__".to_string(),
            find_bin: "".to_string(),
            env_project: None,
        };
        let exports = vec![("FOO", "bar".to_string())];
        let result = args.format_exports(&exports);
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
            let mut buf = vec![];
            let args = SetupEnvArgs {
                mode: "run".to_string(),
                flox_env: PathBuf::from("/env1"),
                env_dirs: "".to_string(),
                shell: shell.to_string(),
                ld_floxlib: "__LINUX_ONLY__".to_string(),
                ldconfig: "__LINUX_ONLY__".to_string(),
                find_bin: "".to_string(),
                env_project: None,
            };
            args.handle_inner(&mut buf).unwrap();
            let output = String::from_utf8(buf).unwrap();
            for line in output.lines() {
                if !line.is_empty() {
                    assert!(line.ends_with(';'), "line missing semicolon: {line}");
                }
            }
        }
    }
}
