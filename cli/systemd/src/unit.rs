//! Support for systemd resource limits and service overrides.
//!
//! This module provides functionality for creating systemd service override
//! files that configure resource limits like CPU quotas, memory limits, etc.
//!
//! Resources:
//! - [systemd.resource-control(5)](https://www.freedesktop.org/software/systemd/man/systemd.resource-control.html)
//! - [systemd.exec(5)](https://www.freedesktop.org/software/systemd/man/systemd.exec.html)
#![allow(clippy::writeln_empty_string)]

use std::collections::HashMap;
use std::fmt::Write;
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error while formatting output: .0")]
    WriteFmt(#[from] std::fmt::Error),
    #[error("error while writing unit file: .0")]
    WriteOutput(#[from] io::Error),
}

/// Represents a systemd service configuration
#[derive(Debug, Clone, Default)]
pub struct ServiceUnit {
    pub unit: Option<Unit>,
    pub service: Option<Service>,
}

/// Unit section configuration
#[derive(Debug, Clone, Default)]
pub struct Unit {
    pub description: Option<String>,
    pub documentation: Option<Vec<String>>,
    pub wants: Option<Vec<String>>,
    pub requires: Option<Vec<String>>,
    pub before: Option<Vec<String>>,
    pub after: Option<Vec<String>>,
}

/// Service section configuration with resource limits
#[derive(Debug, Clone, Default)]
pub struct Service {
    // Basic service configuration
    pub type_: Option<ServiceType>,
    pub exec_start: Option<String>,
    pub exec_start_pre: Option<Vec<String>>,
    pub exec_start_post: Option<Vec<String>>,
    pub exec_stop: Option<String>,
    pub restart: Option<RestartPolicy>,
    pub restart_sec: Option<u32>,
    pub timeout_start_sec: Option<u32>,
    pub timeout_stop_sec: Option<u32>,

    // Resource limits
    pub cpu_quota: Option<String>,
    pub cpu_shares: Option<u32>,
    pub cpu_weight: Option<u32>,
    pub memory_max: Option<String>,
    pub memory_high: Option<String>,
    pub memory_low: Option<String>,
    pub tasks_max: Option<String>,
    pub io_weight: Option<u32>,

    // Process limits (same as nspawn)
    pub limit_cpu: Option<ResourceLimit>,
    pub limit_fsize: Option<ResourceLimit>,
    pub limit_data: Option<ResourceLimit>,
    pub limit_stack: Option<ResourceLimit>,
    pub limit_core: Option<ResourceLimit>,
    pub limit_rss: Option<ResourceLimit>,
    pub limit_nofile: Option<ResourceLimit>,
    pub limit_as: Option<ResourceLimit>,
    pub limit_nproc: Option<ResourceLimit>,
    pub limit_memlock: Option<ResourceLimit>,
    pub limit_locks: Option<ResourceLimit>,
    pub limit_sigpending: Option<ResourceLimit>,
    pub limit_msgqueue: Option<ResourceLimit>,
    pub limit_nice: Option<ResourceLimit>,
    pub limit_rtprio: Option<ResourceLimit>,
    pub limit_rttime: Option<ResourceLimit>,

    // Security settings
    pub private_tmp: Option<bool>,
    pub protect_system: Option<ProtectSystem>,
    pub protect_home: Option<ProtectHome>,
    pub no_new_privileges: Option<bool>,

    // Environment
    pub environment: Option<HashMap<String, String>>,
    pub environment_file: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub enum ResourceLimit {
    Value(String),
    Range { soft: String, hard: String },
}

#[derive(Debug, Clone)]
pub enum ServiceType {
    Simple,
    Exec,
    Forking,
    Oneshot,
    Dbus,
    Notify,
    Idle,
}

#[derive(Debug, Clone)]
pub enum RestartPolicy {
    No,
    OnSuccess,
    OnFailure,
    OnAbnormal,
    OnWatchdog,
    OnAbort,
    Always,
}

#[derive(Debug, Clone)]
pub enum ProtectSystem {
    No,
    Yes,
    Full,
    Strict,
}

#[derive(Debug, Clone)]
pub enum ProtectHome {
    No,
    Yes,
    ReadOnly,
    Tmpfs,
}

// Implement display traits for enums
impl std::fmt::Display for ServiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceType::Simple => write!(f, "simple"),
            ServiceType::Exec => write!(f, "exec"),
            ServiceType::Forking => write!(f, "forking"),
            ServiceType::Oneshot => write!(f, "oneshot"),
            ServiceType::Dbus => write!(f, "dbus"),
            ServiceType::Notify => write!(f, "notify"),
            ServiceType::Idle => write!(f, "idle"),
        }
    }
}

impl std::fmt::Display for RestartPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestartPolicy::No => write!(f, "no"),
            RestartPolicy::OnSuccess => write!(f, "on-success"),
            RestartPolicy::OnFailure => write!(f, "on-failure"),
            RestartPolicy::OnAbnormal => write!(f, "on-abnormal"),
            RestartPolicy::OnWatchdog => write!(f, "on-watchdog"),
            RestartPolicy::OnAbort => write!(f, "on-abort"),
            RestartPolicy::Always => write!(f, "always"),
        }
    }
}

impl std::fmt::Display for ProtectSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtectSystem::No => write!(f, "no"),
            ProtectSystem::Yes => write!(f, "yes"),
            ProtectSystem::Full => write!(f, "full"),
            ProtectSystem::Strict => write!(f, "strict"),
        }
    }
}

impl std::fmt::Display for ProtectHome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtectHome::No => write!(f, "no"),
            ProtectHome::Yes => write!(f, "yes"),
            ProtectHome::ReadOnly => write!(f, "read-only"),
            ProtectHome::Tmpfs => write!(f, "tmpfs"),
        }
    }
}

fn space_separated_list(items: &[String]) -> String {
    items.join(" ")
}

fn map_values(field_name: &str, items: &HashMap<String, String>) -> Result<String, Error> {
    let mut buf = String::new();
    for (key, value) in items.iter() {
        writeln!(&mut buf, "{field_name}={key}={value}")?;
    }
    Ok(buf)
}

pub(crate) trait UnitFmt<T: io::Write> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error>;
}

impl<T: io::Write> UnitFmt<T> for Option<bool> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            if *value {
                writeln!(output, "{name}=yes")?;
            } else {
                writeln!(output, "{name}=no")?;
            }
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<String> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<Vec<String>> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(values) = self {
            let joined = space_separated_list(values);
            writeln!(output, "{name}={joined}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<HashMap<String, String>> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(values) = self {
            let joined = map_values(name, values)?;
            write!(output, "{joined}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<ResourceLimit> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(limit) = self {
            match limit {
                ResourceLimit::Value(value) => writeln!(output, "{name}={value}")?,
                ResourceLimit::Range { soft, hard } => writeln!(output, "{name}={soft}:{hard}")?,
            }
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<u128> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<u16> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

// Implement UnitFmt for the enums
impl<T: io::Write> UnitFmt<T> for Option<ServiceType> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<RestartPolicy> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<ProtectSystem> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<ProtectHome> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

impl<T: io::Write> UnitFmt<T> for Option<u32> {
    fn unit_fmt(&self, name: &str, output: &mut T) -> Result<(), Error> {
        if let Some(value) = self {
            writeln!(output, "{name}={value}")?;
        }
        Ok(())
    }
}

/// Write a systemd service unit file
pub fn write_service_unit(output: &mut impl io::Write, config: &ServiceUnit) -> Result<(), Error> {
    if let Some(ref unit) = config.unit {
        write_unit_section(output, unit)?;
    }
    if let Some(ref service) = config.service {
        write_service_section(output, service)?;
    }
    Ok(())
}

fn write_unit_section(output: &mut impl io::Write, unit_config: &Unit) -> Result<(), Error> {
    writeln!(output, "[Unit]")?;
    unit_config.description.unit_fmt("Description", output)?;
    unit_config
        .documentation
        .unit_fmt("Documentation", output)?;
    unit_config.wants.unit_fmt("Wants", output)?;
    unit_config.requires.unit_fmt("Requires", output)?;
    unit_config.before.unit_fmt("Before", output)?;
    unit_config.after.unit_fmt("After", output)?;
    writeln!(output, "")?;
    Ok(())
}

fn write_service_section(
    output: &mut impl io::Write,
    service_config: &Service,
) -> Result<(), Error> {
    writeln!(output, "[Service]")?;

    // Basic service configuration
    service_config.type_.unit_fmt("Type", output)?;
    service_config.exec_start.unit_fmt("ExecStart", output)?;
    service_config
        .exec_start_pre
        .unit_fmt("ExecStartPre", output)?;
    service_config
        .exec_start_post
        .unit_fmt("ExecStartPost", output)?;
    service_config.exec_stop.unit_fmt("ExecStop", output)?;
    service_config.restart.unit_fmt("Restart", output)?;
    service_config.restart_sec.unit_fmt("RestartSec", output)?;
    service_config
        .timeout_start_sec
        .unit_fmt("TimeoutStartSec", output)?;
    service_config
        .timeout_stop_sec
        .unit_fmt("TimeoutStopSec", output)?;

    // Resource limits
    service_config.cpu_quota.unit_fmt("CPUQuota", output)?;
    service_config.cpu_shares.unit_fmt("CPUShares", output)?;
    service_config.cpu_weight.unit_fmt("CPUWeight", output)?;
    service_config.memory_max.unit_fmt("MemoryMax", output)?;
    service_config.memory_high.unit_fmt("MemoryHigh", output)?;
    service_config.memory_low.unit_fmt("MemoryLow", output)?;
    service_config.tasks_max.unit_fmt("TasksMax", output)?;
    service_config.io_weight.unit_fmt("IOWeight", output)?;

    // Process limits
    service_config.limit_cpu.unit_fmt("LimitCPU", output)?;
    service_config.limit_fsize.unit_fmt("LimitFSIZE", output)?;
    service_config.limit_data.unit_fmt("LimitDATA", output)?;
    service_config.limit_stack.unit_fmt("LimitSTACK", output)?;
    service_config.limit_core.unit_fmt("LimitCORE", output)?;
    service_config.limit_rss.unit_fmt("LimitRSS", output)?;
    service_config
        .limit_nofile
        .unit_fmt("LimitNOFILE", output)?;
    service_config.limit_as.unit_fmt("LimitAS", output)?;
    service_config.limit_nproc.unit_fmt("LimitNPROC", output)?;
    service_config
        .limit_memlock
        .unit_fmt("LimitMEMLOCK", output)?;
    service_config.limit_locks.unit_fmt("LimitLOCKS", output)?;
    service_config
        .limit_sigpending
        .unit_fmt("LimitSIGPENDING", output)?;
    service_config
        .limit_msgqueue
        .unit_fmt("LimitMSGQUEUE", output)?;
    service_config.limit_nice.unit_fmt("LimitNICE", output)?;
    service_config
        .limit_rtprio
        .unit_fmt("LimitRTPRIO", output)?;
    service_config
        .limit_rttime
        .unit_fmt("LimitRTTIME", output)?;

    // Security settings
    service_config.private_tmp.unit_fmt("PrivateTmp", output)?;
    service_config
        .protect_system
        .unit_fmt("ProtectSystem", output)?;
    service_config
        .protect_home
        .unit_fmt("ProtectHome", output)?;
    service_config
        .no_new_privileges
        .unit_fmt("NoNewPrivileges", output)?;

    // Environment
    service_config.environment.unit_fmt("Environment", output)?;
    service_config
        .environment_file
        .unit_fmt("EnvironmentFile", output)?;

    writeln!(output, "")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    #[test]
    fn test_service_override_with_cpu_quota() {
        let mut output = Vec::new();
        let override_config = ServiceUnit {
            service: Some(Service {
                cpu_quota: Some("50%".to_string()),
                memory_max: Some("2G".to_string()),
                tasks_max: Some("512".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        write_service_unit(&mut output, &override_config).unwrap();
        let result = String::from_utf8(output).unwrap();

        let expected = indoc! {"
            [Service]
            CPUQuota=50%
            MemoryMax=2G
            TasksMax=512

        "};
        assert_eq!(result, expected);
    }

    #[test]
    fn test_service_override_with_security() {
        let mut output = Vec::new();
        let override_config = ServiceUnit {
            service: Some(Service {
                private_tmp: Some(true),
                protect_system: Some(ProtectSystem::Full),
                protect_home: Some(ProtectHome::ReadOnly),
                no_new_privileges: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        write_service_unit(&mut output, &override_config).unwrap();
        let result = String::from_utf8(output).unwrap();

        let expected = indoc! {"
            [Service]
            PrivateTmp=yes
            ProtectSystem=full
            ProtectHome=read-only
            NoNewPrivileges=yes

        "};
        assert_eq!(result, expected);
    }

    #[test]
    fn test_full_service_override() {
        let mut output = Vec::new();
        let override_config = ServiceUnit {
            unit: Some(Unit {
                description: Some("Custom service description".to_string()),
                after: Some(vec!["network.target".to_string()]),
                ..Default::default()
            }),
            service: Some(Service {
                type_: Some(ServiceType::Notify),
                restart: Some(RestartPolicy::Always),
                restart_sec: Some(5),
                cpu_quota: Some("75%".to_string()),
                memory_max: Some("4G".to_string()),
                ..Default::default()
            }),
        };

        write_service_unit(&mut output, &override_config).unwrap();
        let result = String::from_utf8(output).unwrap();

        let expected = indoc! {"
            [Unit]
            Description=Custom service description
            After=network.target

            [Service]
            Type=notify
            Restart=always
            RestartSec=5
            CPUQuota=75%
            MemoryMax=4G

        "};
        assert_eq!(result, expected);
    }
}
