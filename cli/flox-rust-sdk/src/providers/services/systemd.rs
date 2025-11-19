//! Backend for persisting Flox services to systemd.

use std::io;

use shell_escape::escape;
use systemd::unit::ServiceUnit;

use crate::models::environment_ref::ActivateEnvironmentRef;
use crate::models::manifest::typed::{Inner, ServiceDescriptor};

/// Wrap a command with Flox activation.
//
// TODO: set or allow configuration of activation mode?
// TODO: use store path for bash?
fn wrap_command(env_ref: &ActivateEnvironmentRef, command: &str) -> String {
    let activate_arg = env_ref.activate_target_arg();
    let bash_script_arg = {
        // Workaround logging association for user services:
        // https://github.com/systemd/systemd/issues/2913#issuecomment-3289916490
        let logging_prefix = "exec 1> >(cat); exec 2> >(cat >&2); ";

        // Replace newline characters with literal `\n` sequences so that multi-line
        // commands can be quoted and fit in a single systemd directive line.
        let escaped_newlines = command.replace('\n', r"\n");

        escape(format!("{logging_prefix}{escaped_newlines}").into())
    };

    format!(
        r#"flox activate {} -- exec bash -c {}"#,
        activate_arg, bash_script_arg,
    )
}

/// Wrap multiple commands with Flox activation.
fn wrap_commands(env_ref: &ActivateEnvironmentRef, commands: Vec<String>) -> Vec<String> {
    commands
        .iter()
        .map(|cmd| wrap_command(env_ref, cmd))
        .collect()
}

/// Context for converting a ServiceDescriptor to a ServiceUnit.
pub struct ServiceUnitContext<'a> {
    pub env_ref: &'a ActivateEnvironmentRef,
    pub descriptor: &'a ServiceDescriptor,
}

impl<'a> ServiceUnitContext<'a> {
    /// Merge environment variables from descriptor and systemd service.
    /// Descriptor vars are set first, then systemd vars take precedence.
    fn merge_env_vars(
        &self,
        systemd_env: Option<std::collections::BTreeMap<String, String>>,
    ) -> Option<std::collections::BTreeMap<String, String>> {
        let mut env = self
            .descriptor
            .vars
            .as_ref()
            .map(|v| v.inner().clone())
            .unwrap_or_default();

        if let Some(e) = systemd_env {
            env.extend(e)
        }

        Some(env).filter(|e| !e.is_empty())
    }
}

/// Convert a `ServiceUnitContext` (and thus `typed::ServiceDescriptor`) into a
/// `systemd::ServiceUnit` by merging the descriptor's fields with any explicit
/// systemd configuration.
//
// TODO: Set `WorkingDirectory` so that project-relative paths work?
// TODO: Inject `Requires` to ensure that Nix is started/mounted at boot?
impl<'a> From<ServiceUnitContext<'a>> for ServiceUnit {
    fn from(ctx: ServiceUnitContext<'a>) -> Self {
        let descriptor = ctx.descriptor;
        let base_config = descriptor.systemd.clone().unwrap_or_default();
        let base_service = base_config.service.unwrap_or_default();

        let type_ = base_service.type_.or_else(|| {
            descriptor
                .is_daemon
                .and_then(|is_daemon| is_daemon.then_some(systemd::unit::ServiceType::Forking))
        });

        let exec_start = base_service
            .exec_start
            .or_else(|| Some(descriptor.command.clone()))
            .map(|cmd| wrap_command(ctx.env_ref, &cmd));
        let exec_stop = base_service
            .exec_stop
            .or_else(|| descriptor.shutdown.as_ref().map(|s| s.command.clone()))
            .map(|cmd| wrap_command(ctx.env_ref, &cmd));

        let exec_start_pre = base_service
            .exec_start_pre
            .map(|cmds| wrap_commands(ctx.env_ref, cmds));
        let exec_start_post = base_service
            .exec_start_post
            .map(|cmds| wrap_commands(ctx.env_ref, cmds));

        let environment = ctx.merge_env_vars(base_service.environment);

        let service = systemd::unit::Service {
            type_,
            exec_start,
            exec_stop,
            exec_start_pre,
            exec_start_post,
            environment,
            ..base_service
        };

        ServiceUnit {
            service: Some(service),
            ..base_config
        }
    }
}

/// Render a ServiceDescriptor to a systemd unit file.
pub fn render_systemd_unit_file(
    env_ref: &ActivateEnvironmentRef,
    descriptor: &ServiceDescriptor,
    output: &mut impl io::Write,
) -> Result<(), systemd::unit::Error> {
    let ctx = ServiceUnitContext {
        env_ref,
        descriptor,
    };
    let service_unit = ServiceUnit::from(ctx);
    systemd::unit::write_service_unit(output, &service_unit)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use systemd::unit::{Service, ServiceType, ServiceUnit, Unit};

    use super::*;
    use crate::models::environment_ref::RemoteEnvironmentRef;
    use crate::models::manifest::typed::{ServiceDescriptor, ServiceShutdown, Vars};

    fn project_env_ref() -> ActivateEnvironmentRef {
        ActivateEnvironmentRef::Local(Path::new("/test/env").to_path_buf())
    }

    fn remote_env_ref() -> ActivateEnvironmentRef {
        ActivateEnvironmentRef::Remote(RemoteEnvironmentRef::new("owner", "name").unwrap())
    }

    // table driven tests with a test case type for wrap_command
    #[test]
    fn wrap_command_table_tests() {
        struct TestCase {
            description: &'static str,
            env_ref: ActivateEnvironmentRef,
            input: String,
            expected: String,
        }

        let test_cases = vec![
            TestCase {
                description: "project env with simple command",
                env_ref: project_env_ref(),
                input: "true".to_string(),
                expected: r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); true'"#.to_string(),
            },
            TestCase {
                description: "remote env with simple command",
                env_ref: remote_env_ref(),
                input: "true".to_string(),
                expected: r#"flox activate -r owner/name -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); true'"#.to_string(),
            },
            TestCase {
                description: "project env path with spaces",
                env_ref: ActivateEnvironmentRef::Local(Path::new("/dir with/spaces in").to_path_buf()),
                input: "echo command with spaces in".to_string(),
                expected: r#"flox activate -d '/dir with/spaces in' -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); echo command with spaces in'"#.to_string(),
            },
            TestCase {
                description: "command with quotes",
                env_ref: project_env_ref(),
                input: "echo 'this is quoted'".to_string(),
                expected: r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); echo '\''this is quoted'\'''"#.to_string(),
            },
            TestCase {
                description: "command with multi-line shell script",
                env_ref: project_env_ref(),
                input: indoc! {"
                    while true; do
                      echo hello
                      sleep 2
                    done
                "}.to_string(),
                expected: r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); while true; do\n  echo hello\n  sleep 2\ndone\n'"#.to_string(),
            },
        ];

        for case in test_cases {
            assert_eq!(
                wrap_command(&case.env_ref, &case.input),
                case.expected,
                "wrap_command test case: {}",
                case.description
            );
        }
    }

    #[test]
    fn from_service_descriptor_minimal() {
        let descriptor = ServiceDescriptor {
            command: "echo hello".to_string(),
            vars: None,
            is_daemon: None,
            shutdown: None,
            systemd: None,
            systems: None,
        };

        let ctx = ServiceUnitContext {
            env_ref: &project_env_ref(),
            descriptor: &descriptor,
        };

        assert_eq!(ServiceUnit::from(ctx), ServiceUnit {
            unit: None,
            service: Some(Service {
                exec_start: Some(
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); echo hello'"#.to_string()
                ),
                ..Default::default()
            }),
        });
    }

    // All descriptor fields set and no systemd fields set.
    #[test]
    fn from_service_descriptor_all() {
        let vars = BTreeMap::from_iter(vec![
            ("FOO".to_string(), "foo".to_string()),
            ("BAR".to_string(), "bar".to_string()),
        ]);

        let descriptor = ServiceDescriptor {
            command: "start-command".to_string(),
            vars: Some(Vars(vars.clone())),
            is_daemon: Some(true),
            shutdown: Some(ServiceShutdown {
                command: "stop-command".to_string(),
            }),
            systemd: None,
            systems: None,
        };

        let ctx = ServiceUnitContext {
            env_ref: &project_env_ref(),
            descriptor: &descriptor,
        };

        assert_eq!(ServiceUnit::from(ctx), ServiceUnit {
            unit: None,
            service: Some(Service {
                type_: Some(ServiceType::Forking),
                exec_start: Some(
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); start-command'"#.to_string()
                ),
                exec_stop: Some(
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); stop-command'"#.to_string()
                ),
                environment: Some(vars),
                ..Default::default()
            }),
        });
    }

    // Descriptor and systemd fields set, with systemd fields taking precedence.
    // Vars are merged, with systemd vars taking precedence.
    #[test]
    fn from_service_descriptor_precedence() {
        let descriptor = ServiceDescriptor {
            command: "start-descriptor".to_string(), // overridden
            vars: Some(Vars(BTreeMap::from_iter(vec![
                ("DESCRIPTOR_ONLY".to_string(), "from-descriptor".to_string()),
                // overridden by systemd.service.envrionment
                ("SHARED_KEY".to_string(), "from-descriptor".to_string()),
            ]))),
            // overridden by systemd.service.type
            is_daemon: Some(true),
            shutdown: Some(ServiceShutdown {
                // overridden by systemd.service.exec_start_post
                command: "stop-descriptor".to_string(),
            }),
            systemd: Some(ServiceUnit {
                unit: Some(Unit {
                    description: Some("some service".to_string()),
                    ..Default::default()
                }),
                service: Some(Service {
                    type_: Some(ServiceType::Notify),
                    exec_start: Some("start-command".to_string()),
                    exec_start_pre: Some(vec!["pre-command".to_string()]),
                    exec_start_post: Some(vec!["post-command".to_string()]),
                    exec_stop: Some("stop-command".to_string()),
                    environment: Some(BTreeMap::from_iter(vec![
                        ("SHARED_KEY".to_string(), "from-systemd".to_string()),
                        ("SYSTEMD_ONLY".to_string(), "from-systemd".to_string()),
                    ])),
                    ..Default::default()
                }),
            }),
            systems: None,
        };

        let ctx = ServiceUnitContext {
            env_ref: &project_env_ref(),
            descriptor: &descriptor,
        };

        assert_eq!(ServiceUnit::from(ctx), ServiceUnit {
            unit: Some(Unit {
                description: Some("some service".to_string()),
                ..Default::default()
            }),
            service: Some(Service {
                type_: Some(ServiceType::Notify),
                exec_start: Some(
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); start-command'"#.to_string()
                ),
                exec_start_pre: Some(vec![
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); pre-command'"#.to_string()
                ]),
                exec_start_post: Some(vec![
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); post-command'"#.to_string()
                ]),
                exec_stop: Some(
                    r#"flox activate -d /test/env -- exec bash -c 'exec 1> >(cat); exec 2> >(cat >&2); stop-command'"#.to_string()
                ),
                environment: Some(BTreeMap::from_iter(vec![
                    ("SHARED_KEY".to_string(), "from-systemd".to_string()),
                    ("DESCRIPTOR_ONLY".to_string(), "from-descriptor".to_string()),
                    ("SYSTEMD_ONLY".to_string(), "from-systemd".to_string()),
                ])),
                ..Default::default()
            }),
        });
    }
}
