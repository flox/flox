//! Explicit PATH-only setup; ordinary activation helpers remain separate.
use std::borrow::Cow;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::{env, fs};

use flox_core::write_atomically_with_permissions;
use indoc::formatdoc;
use serde::Serialize;
use shell_gen::Shell;

use super::{GlobalError, GlobalPaths, GlobalResult};
use crate::utils::detect_shell::detect_shell_for_subshell;

const BEGIN: &str = "# >>> Flox global commands >>>";
const END: &str = "# <<< Flox global commands <<<";

#[derive(Clone, Debug, Serialize)]
pub struct ShellReport {
    pub shell: String,
    pub script: String,
    pub changed_files: Vec<PathBuf>,
    pub errors: Vec<GlobalError>,
}

pub fn setup(
    paths: &GlobalPaths,
    selected: Option<Shell>,
    print_only: bool,
) -> GlobalResult<ShellReport> {
    let shell = selected.unwrap_or_else(|| detect_shell_for_subshell().into());
    let script = script(paths, shell)?;
    let mut report = ShellReport {
        shell: shell.to_string(),
        script,
        changed_files: vec![],
        errors: vec![],
    };
    if print_only {
        return Ok(report);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| GlobalError::new("shell_setup", "HOME is not set", None))?;
    let files = match shell {
        Shell::Bash => {
            let profile = [".bash_profile", ".bash_login", ".profile"]
                .into_iter()
                .map(|name| home.join(name))
                .find(|path| path.exists())
                .unwrap_or_else(|| home.join(".profile"));
            vec![home.join(".bashrc"), profile]
        },
        Shell::Zsh => {
            let root = env::var_os("ZDOTDIR").map(PathBuf::from).unwrap_or(home);
            vec![root.join(".zshrc"), root.join(".zprofile")]
        },
        Shell::Fish => {
            let root = env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"));
            vec![root.join("fish/conf.d/flox-global.fish")]
        },
        Shell::Tcsh => vec![home.join(".tcshrc")],
    };
    for file in files {
        match update_file(&file, &report.script) {
            Ok(true) => report.changed_files.push(file),
            Ok(false) => {},
            Err(error) => {
                report.errors.push(error);
                break;
            },
        }
    }
    Ok(report)
}

pub fn script(paths: &GlobalPaths, shell: Shell) -> GlobalResult<String> {
    let quoted = |name: &str| -> GlobalResult<String> {
        let path = paths.data_dir.join(name);
        let text = path.to_str().ok_or_else(|| {
            GlobalError::at(
                "unsupported_path",
                &path,
                "Shell PATH entries must be valid UTF-8",
            )
        })?;
        if text.contains([':', '\n', '\r']) {
            return Err(GlobalError::at(
                "unsupported_path",
                &path,
                "Shell PATH entries cannot contain colons or line breaks",
            ));
        }
        Ok(shell_escape::escape(Cow::Borrowed(text)).into_owned())
    };
    let bin = quoted("bin")?;
    let sbin = quoted("sbin")?;
    let contents = match shell {
        Shell::Bash | Shell::Zsh => formatdoc! {"
            _flox_global_path=
            _flox_global_sep=
            _flox_global_rest=${{PATH-}}
            while :; do
              case $_flox_global_rest in
                *:*) _flox_global_part=${{_flox_global_rest%%:*}}; _flox_global_rest=${{_flox_global_rest#*:}}; _flox_global_more=1 ;;
                *) _flox_global_part=$_flox_global_rest; _flox_global_more=0 ;;
              esac
              case $_flox_global_part in
                {bin}|{sbin}) ;;
                *) _flox_global_path=\"${{_flox_global_path}}${{_flox_global_sep}}${{_flox_global_part}}\"; _flox_global_sep=: ;;
              esac
              [ \"$_flox_global_more\" = 1 ] || break
            done
            export PATH=\"${{_flox_global_path}}${{_flox_global_sep}}\"{bin}:{sbin}
            unset _flox_global_path _flox_global_sep _flox_global_rest _flox_global_part _flox_global_more
        "},
        Shell::Fish => formatdoc! {"
            set -l _flox_global_path
            for _flox_global_part in $PATH
              if test \"$_flox_global_part\" != {bin}; and test \"$_flox_global_part\" != {sbin}
                set -a _flox_global_path \"$_flox_global_part\"
              end
            end
            set -gx PATH $_flox_global_path {bin} {sbin}
            set -e _flox_global_path
            set -e _flox_global_part
        "},
        Shell::Tcsh => formatdoc! {"
            set _flox_global_path = ()
            foreach _flox_global_part ( $path:q )
              if ( \"$_flox_global_part\" != {bin} && \"$_flox_global_part\" != {sbin} ) then
                set _flox_global_path = ( $_flox_global_path:q \"$_flox_global_part\" )
              endif
            end
            set path = ( $_flox_global_path:q {bin} {sbin} )
            unset _flox_global_path _flox_global_part
        "},
    };
    Ok(format!("{BEGIN}\n{contents}{END}\n"))
}

fn update_file(path: &Path, script: &str) -> GlobalResult<bool> {
    let permissions = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => {
            return Err(GlobalError::at(
                "shell_setup",
                path,
                "Startup file is a symlink or other non-regular entry; add the printed script explicitly",
            ));
        },
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(GlobalError::at("shell_setup", path, error)),
    };
    let old = if permissions.is_some() {
        fs::read_to_string(path).map_err(|error| GlobalError::at("shell_setup", path, error))?
    } else {
        String::new()
    };
    if [BEGIN, END].iter().any(|marker| {
        old.matches(marker).count() != old.lines().filter(|line| line == marker).count()
    }) {
        return Err(GlobalError::at(
            "shell_setup",
            path,
            "Flox global setup markers must occupy complete lines",
        ));
    }
    let new = match (old.matches(BEGIN).count(), old.matches(END).count()) {
        (0, 0) => format!(
            "{old}{}{script}",
            if old.is_empty() || old.ends_with('\n') {
                ""
            } else {
                "\n"
            }
        ),
        (1, 1) => {
            let start = old.find(BEGIN).expect("marker counted");
            let end = old.find(END).expect("marker counted") + END.len();
            if end <= start {
                return Err(GlobalError::at(
                    "shell_setup",
                    path,
                    "Malformed Flox global setup block",
                ));
            }
            let suffix = old[end..].strip_prefix('\n').unwrap_or(&old[end..]);
            format!("{}{script}{suffix}", &old[..start])
        },
        _ => {
            return Err(GlobalError::at(
                "shell_setup",
                path,
                "Ambiguous Flox global setup blocks",
            ));
        },
    };
    if new == old {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| GlobalError::at("shell_setup", parent, error))?;
    }
    write_atomically_with_permissions(path, new, permissions)
        .map_err(|error| GlobalError::at("shell_setup", path, error))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn setup_preserves_other_content_and_permissions_on_repeat() {
        let temp = tempdir().unwrap();
        let file = temp.path().join(".bashrc");
        fs::write(&file, "# user settings\nexport KEEP_ME=yes").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        let script = script(
            &GlobalPaths {
                data_dir: temp.path().join("data with spaces"),
            },
            Shell::Bash,
        )
        .unwrap();
        assert!(update_file(&file, &script).unwrap());
        let saved = fs::read_to_string(&file).unwrap();
        assert!(!update_file(&file, &script).unwrap());
        assert_eq!(
            (
                fs::read_to_string(&file).unwrap(),
                fs::metadata(&file).unwrap().permissions().mode() & 0o777
            ),
            (
                format!("# user settings\nexport KEEP_ME=yes\n{script}"),
                0o640
            )
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), saved);
    }

    #[test]
    fn ambiguous_blocks_and_symlink_startup_files_are_preserved() {
        let temp = tempdir().unwrap();
        let file = temp.path().join(".zshrc");
        for contents in [
            format!("prefix{BEGIN}\n{END}\n"),
            format!("{END}\n{BEGIN}\n"),
            format!("{BEGIN}\n{BEGIN}\n{END}\n"),
        ] {
            fs::write(&file, &contents).unwrap();
            assert_eq!(
                update_file(&file, "replacement").unwrap_err().category,
                "shell_setup"
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), contents);
        }
        let link = temp.path().join(".bashrc");
        symlink(&file, &link).unwrap();
        let before = fs::read_to_string(&file).unwrap();
        assert_eq!(
            update_file(&link, "replacement").unwrap_err().category,
            "shell_setup"
        );
        assert_eq!(
            (
                fs::read_link(&link).unwrap(),
                fs::read_to_string(&file).unwrap()
            ),
            (file, before)
        );
    }
}
