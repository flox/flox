//! Parsing and replay of bash-envtrace trace files.
//!
//! The activate script runs under a bash patched to record every
//! environment-visible variable mutation to a trace file. The format
//! specification is the header comment of the vendored
//! `pkgs/flox-interpreter/bash-5.3-envtrace.patch` (its source repo,
//! github.com/flox/bash-envtrace, is not public). Each record is one line of six
//! fields separated by the unit separator byte `0x1f`:
//!
//! ```text
//! <timestamp> <op> <exported> <name> <old> <operand>
//! ```
//!
//! Names and values are escaped so a record occupies exactly one line
//! (`\\`, `\n`, `\r`, `\xHH`); values are prefixed with `:` when the
//! variable existed or are the single byte `@` when it did not.
//!
//! Unlike a before/after environment diff, a trace records *how* each value
//! was built: `prepend`/`append` records carry only the delta, so replaying
//! the trace onto a different shell's environment extends that shell's own
//! value instead of clobbering it with the value captured in the shell that
//! ran the activation.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::env_diff::EnvDiff;

/// Trace file written by the activate script into the start state dir.
pub const ENV_TRACE_LOG: &str = "envtrace.log";

/// The environment mutations an activation performed, as recorded by the
/// activate script.
///
/// Holds the parsed records rather than the values they produce: what the
/// activation *did* only becomes a set of variables to apply once
/// [`EnvTrace::generate_diff`] resolves it against the environment it is
/// being applied to.
#[derive(Debug, Clone)]
pub struct EnvTrace(Vec<TraceRecord>);

impl EnvTrace {
    /// Load the trace an activation left in its start state directory.
    ///
    /// Every start state directory this binary can reach carries a trace: an
    /// activation started by an interpreter that did not write one also wrote
    /// an incompatible `state.json` version, which is refused before we get
    /// here. An unreadable or empty trace is therefore corruption, not an
    /// older format to degrade for.
    pub fn from_state_dir(start_state_dir: impl AsRef<Path>) -> Result<EnvTrace> {
        let trace_log = start_state_dir.as_ref().join(ENV_TRACE_LOG);
        let records = parse_trace_file(&trace_log)?;
        // An empty trace cannot legitimately occur: every activation mode
        // mutates variables inside the window (profile.d alone touches CPATH
        // and friends), so empty means the tracer failed to record — the
        // activate script creates the file before the tracer opens it, and
        // the tracer drops write errors silently.
        if records.is_empty() {
            bail!("{} is empty", trace_log.display());
        }
        Ok(EnvTrace(records))
    }

    /// Generate the diff that, applied to `base_env`, faithfully replays the
    /// trace. `base_env` is the environment the activate script would have
    /// seen had it been started from the shell being activated.
    ///
    /// The replay is semantic rather than literal: prepend/append records
    /// extend `base_env`'s own values instead of replaying the absolute
    /// values the shell that ran the activate script ended up with.
    pub fn generate_diff(&self, base_env: &HashMap<String, String>) -> EnvDiff {
        generate_diff_from_trace(&self.0, base_env)
    }

    /// Build a trace from records, for tests that care how the activation
    /// arrived at its values.
    #[cfg(test)]
    pub fn from_records(records: Vec<TraceRecord>) -> Self {
        EnvTrace(records)
    }

    /// Build a trace that sets `additions` and unsets `deletions` outright,
    /// for tests that only need an activation's net effect.
    #[cfg(test)]
    pub fn from_parts(additions: HashMap<String, String>, deletions: Vec<String>) -> Self {
        EnvTrace(
            additions
                .into_iter()
                .map(|(name, value)| TraceRecord {
                    op: TraceOp::Set,
                    name,
                    old: None,
                    operand: Some(value),
                })
                .chain(deletions.into_iter().map(|name| TraceRecord {
                    op: TraceOp::Unset,
                    name,
                    old: None,
                    operand: None,
                }))
                .collect(),
        )
    }
}

/// Variables ignored during replay, mirroring the default ignores of the
/// reference envtrace-replay/envtrace-unwind tools: bash updates `_` on
/// every command (it is only traced at all when the invoking environment
/// exported it), `SHELLOPTS` membership can flip through an untraceable code
/// path, and the control variables are tracer plumbing that should never
/// appear but are excluded defensively. `PWD`/`OLDPWD` are directory
/// tracking: a hook's `cd`/`pushd` records them even when it restores the
/// directory before returning, and replaying the start shell's cwd into an
/// attaching shell would desynchronize `$PWD` from `pwd`.
const IGNORED_VARS: &[&str] = &[
    "_",
    "SHELLOPTS",
    "PWD",
    "OLDPWD",
    "BASH_ENVTRACE_FILE",
    "BASH_ENVTRACE_FD",
    "BASH_ENVTRACE_RESET",
];

/// Skip `BASH_FUNC_*` variables, which `export -f` or `unset -f` in a hook
/// creates: their names contain `%%`, which is not a valid variable name in
/// the other shells an activation is replayed into.
const BASH_EXPORTED_FUNC_PREFIX: &str = "BASH_FUNC_";

const FIELD_SEPARATOR: char = '\u{1f}';

/// Mutation kind of a trace record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceOp {
    /// Entry appeared in the environment; operand is the full value.
    Set,
    /// Member reassigned a byte-identical value under declared always-reset
    /// intent (`BASH_ENVTRACE_RESET` covered the variable at mutation
    /// time); operand is that value and replay overwrites unconditionally.
    Reset,
    /// Member reassigned a byte-identical value with no declared intent
    /// (the default); operand is that value and replay applies it only
    /// when the target has no value of its own.
    SetIfAbsent,
    /// Member grew at the front; operand is only the added prefix.
    Prepend,
    /// Member grew at the end; operand is only the added suffix.
    Append,
    /// Member changed any other way; operand is the full new value.
    Updated,
    /// Entry removed from the environment.
    Unset,
    /// `FOO=bar cmd` overlay; never mutates the recording shell.
    Tempenv,
}

/// One parsed trace record.
///
/// The timestamp and pre/post export digits are validated during parsing but
/// not retained: replay only needs the operation, the variable, and the
/// operand. The recorded old value is retained for diagnostics but never
/// replayed — it belongs to the start shell's context.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRecord {
    pub op: TraceOp,
    pub name: String,
    /// Full value before the operation; `None` if the variable did not exist.
    pub old: Option<String>,
    /// The op's argument; `None` for `unset`.
    pub operand: Option<String>,
}

/// Net effect of a trace on a single variable.
#[derive(Debug, Clone, PartialEq)]
enum VarEffect {
    /// Ends with an absolute value.
    Set(String),
    /// Removed from the environment.
    Unset,
}

/// Parse the contents of a trace file.
///
/// Parsing is strict: a malformed record is an error, not a skip, because an
/// unparseable trace means the activation record cannot be trusted.
fn parse_trace(contents: &str) -> Result<Vec<TraceRecord>> {
    contents
        .lines()
        .enumerate()
        .map(|(i, line)| {
            parse_record(line).with_context(|| format!("malformed trace record on line {}", i + 1))
        })
        .collect()
}

/// Read and parse a trace file from disk.
fn parse_trace_file(path: impl AsRef<Path>) -> Result<Vec<TraceRecord>> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    parse_trace(&contents).with_context(|| format!("Failed to parse {}", path.display()))
}

fn parse_record(line: &str) -> Result<TraceRecord> {
    let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
    let [timestamp, op, exported, name, old, operand] = fields.as_slice() else {
        bail!("expected 6 fields, found {}", fields.len());
    };

    if timestamp.is_empty() {
        bail!("empty timestamp field");
    }
    let op = match *op {
        "set" => TraceOp::Set,
        "reset" => TraceOp::Reset,
        "set-if-absent" => TraceOp::SetIfAbsent,
        "prepend" => TraceOp::Prepend,
        "append" => TraceOp::Append,
        "updated" => TraceOp::Updated,
        "unset" => TraceOp::Unset,
        "tempenv" => TraceOp::Tempenv,
        other => bail!("unknown op '{other}'"),
    };
    if exported.len() != 2 || !exported.bytes().all(|b| b == b'0' || b == b'1') {
        bail!("invalid exported field '{exported}'");
    }

    let name = unescape(name)?;
    let old = decode_value(old)?;
    let operand = decode_value(operand)?;
    if !matches!(op, TraceOp::Unset) && operand.is_none() {
        bail!("op '{op:?}' requires an operand");
    }

    Ok(TraceRecord {
        op,
        name,
        old,
        operand,
    })
}

/// Decode a value field: `@` means the variable did not exist, `:` followed
/// by the escaped (possibly empty) value means it did.
fn decode_value(field: &str) -> Result<Option<String>> {
    match field.strip_prefix(':') {
        Some(escaped) => Ok(Some(unescape(escaped)?)),
        None if field == "@" => Ok(None),
        None => Err(anyhow!("invalid value field '{field}'")),
    }
}

/// Decode the trace escaping: `\\`, `\n`, `\r`, and `\xHH`. Unknown or
/// truncated escapes are hard errors.
fn unescape(escaped: &str) -> Result<String> {
    let mut out = String::with_capacity(escaped.len());
    let mut chars = escaped.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                if hex.len() != 2 {
                    bail!("truncated \\x escape");
                }
                let byte = u8::from_str_radix(&hex, 16)
                    .map_err(|_| anyhow!("invalid \\x escape '\\x{hex}'"))?;
                // The writer only escapes control bytes (< 0x20) and the
                // field separator (0x1f); anything else would widen to a
                // different byte sequence when pushed as a char, so treat
                // it as corruption rather than decode it wrong.
                if byte >= 0x20 {
                    bail!("\\x{hex} escapes a byte the tracer never escapes");
                }
                out.push(byte as char);
            },
            Some(other) => bail!("unknown escape '\\{other}'"),
            None => bail!("trailing backslash"),
        }
    }
    Ok(out)
}

/// Generate the diff that, applied to a base environment, faithfully
/// replays a trace.
///
/// Application is semantic, not blind overwrite: `set`/`updated`/`reset`
/// assign their operand, `prepend`/`append` apply their delta to the value
/// the base environment currently holds (an empty base when it has none —
/// the recorded old value is the start shell's and is never replayed),
/// `unset` removes, `tempenv` is a no-op, and `set-if-absent` (a
/// same-value assignment without declared reset intent) is applied only
/// when the base has no value at all.
fn generate_diff_from_trace(
    records: &[TraceRecord],
    base_env: &HashMap<String, String>,
) -> EnvDiff {
    let mut effects: HashMap<String, VarEffect> = HashMap::new();

    for record in records {
        if IGNORED_VARS.contains(&record.name.as_str())
            || record.name.starts_with(BASH_EXPORTED_FUNC_PREFIX)
        {
            continue;
        }
        let current = match effects.get(&record.name) {
            Some(VarEffect::Set(value)) => Some(value.clone()),
            Some(VarEffect::Unset) => None,
            None => base_env.get(&record.name).cloned(),
        };
        let effect = match record.op {
            TraceOp::Tempenv => continue,
            // Same-value assignments carry their intent in the op since
            // trace format v8. A `set-if-absent` (no intent declared, the
            // default for user hook and profile.d code) defers to the
            // target: a shell that holds its own value keeps it, one
            // without the variable gets the recorded value. A `reset`
            // (declared via BASH_ENVTRACE_RESET, as the manifest `[vars]`
            // application does) is an unconditional overwrite, identical
            // to `set`/`updated`.
            TraceOp::SetIfAbsent => {
                if current.is_some() {
                    continue;
                }
                VarEffect::Set(record.operand.clone().expect("validated at parse time"))
            },
            TraceOp::Set | TraceOp::Reset | TraceOp::Updated => {
                VarEffect::Set(record.operand.clone().expect("validated at parse time"))
            },
            TraceOp::Unset => VarEffect::Unset,
            TraceOp::Prepend | TraceOp::Append => {
                let delta = record.operand.clone().expect("validated at parse time");
                // When the target has no value the base is EMPTY, not the
                // recorded old value: the old value is the *start* shell's
                // and replaying it would leak that shell's stack into a
                // target that never had the variable — the exact class of
                // bug the trace exists to eliminate.
                let base = current.unwrap_or_default();
                match record.op {
                    TraceOp::Prepend => VarEffect::Set(format!("{delta}{base}")),
                    _ => VarEffect::Set(format!("{base}{delta}")),
                }
            },
        };
        effects.insert(record.name.clone(), effect);
    }

    let mut env_diff = EnvDiff::default();
    for (name, effect) in effects {
        match effect {
            VarEffect::Set(value) => {
                env_diff.additions.insert(name, value);
            },
            VarEffect::Unset => {
                env_diff.deletions.push(name);
            },
        }
    }
    env_diff
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const US: char = '\u{1f}';

    fn record(fields: [&str; 6]) -> String {
        fields.join(&US.to_string())
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn diff(additions: &[(&str, &str)], deletions: &[&str]) -> EnvDiff {
        EnvDiff::from_parts(
            env(additions),
            deletions.iter().map(|d| d.to_string()).collect(),
        )
    }

    #[test]
    fn parses_readme_example() {
        let contents = [
            record(["1786544639.262486", "set", "01", "FOO", "@", ":base"]),
            record([
                "1786544639.262489",
                "append",
                "11",
                "FOO",
                ":base",
                "::extra",
            ]),
            record(["1786544639.262525", "set", "01", "MYPATH", "@", ":/bin"]),
            record([
                "1786544639.262534",
                "append",
                "11",
                "MYPATH",
                ":/bin",
                "::/usr/sbin",
            ]),
            record([
                "1786544639.262541",
                "unset",
                "10",
                "FOO",
                ":base:extra",
                "@",
            ]),
        ]
        .join("\n");

        let records = parse_trace(&contents).unwrap();
        assert_eq!(records, vec![
            TraceRecord {
                op: TraceOp::Set,
                name: "FOO".to_string(),
                old: None,
                operand: Some("base".to_string()),
            },
            TraceRecord {
                op: TraceOp::Append,
                name: "FOO".to_string(),
                old: Some("base".to_string()),
                operand: Some(":extra".to_string()),
            },
            TraceRecord {
                op: TraceOp::Set,
                name: "MYPATH".to_string(),
                old: None,
                operand: Some("/bin".to_string()),
            },
            TraceRecord {
                op: TraceOp::Append,
                name: "MYPATH".to_string(),
                old: Some("/bin".to_string()),
                operand: Some(":/usr/sbin".to_string()),
            },
            TraceRecord {
                op: TraceOp::Unset,
                name: "FOO".to_string(),
                old: Some("base:extra".to_string()),
                operand: None,
            },
        ]);
    }

    #[test]
    fn parses_set_if_absent_op() {
        // v8 splits same-value assignments by declared intent: undeclared
        // ones record `set-if-absent`, declared ones keep `reset`.
        let contents = [
            record(["1.0", "set-if-absent", "11", "EDITOR", ":vim", ":vim"]),
            record(["1.1", "reset", "11", "FOO", ":defined", ":defined"]),
        ]
        .join("\n");

        let records = parse_trace(&contents).unwrap();
        assert_eq!(records, vec![
            TraceRecord {
                op: TraceOp::SetIfAbsent,
                name: "EDITOR".to_string(),
                old: Some("vim".to_string()),
                operand: Some("vim".to_string()),
            },
            TraceRecord {
                op: TraceOp::Reset,
                name: "FOO".to_string(),
                old: Some("defined".to_string()),
                operand: Some("defined".to_string()),
            },
        ]);
    }

    #[test]
    fn parses_escapes_and_empty_values() {
        let contents = record([
            "1.0",
            "set",
            "01",
            "MULTI",
            ":",
            ":line1\\nline2\\r\\\\slash\\x1fus",
        ]);
        let records = parse_trace(&contents).unwrap();
        assert_eq!(records, vec![TraceRecord {
            op: TraceOp::Set,
            name: "MULTI".to_string(),
            old: Some("".to_string()),
            operand: Some("line1\nline2\r\\slash\u{1f}us".to_string()),
        }]);
    }

    #[test]
    fn rejects_malformed_records() {
        // Too few fields.
        assert!(parse_trace(&["1.0", "set", "01", "FOO", ":x"].join(&US.to_string())).is_err());
        // Unknown op.
        assert!(parse_trace(&record(["1.0", "export", "01", "FOO", "@", ":x"])).is_err());
        // Invalid exported digits.
        assert!(parse_trace(&record(["1.0", "set", "2", "FOO", "@", ":x"])).is_err());
        // Bare value without ':' or '@'.
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", "x"])).is_err());
        // Unknown escape.
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", ":\\q"])).is_err());
        // Truncated \x escape.
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", ":\\x1"])).is_err());
        // \x escape for a byte the tracer never escapes (>= 0x20): decoding
        // it would widen the byte, so it is treated as corruption.
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", ":\\xff"])).is_err());
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", ":\\x20"])).is_err());
        // Missing operand for a value op.
        assert!(parse_trace(&record(["1.0", "set", "01", "FOO", "@", "@"])).is_err());
    }

    #[test]
    fn generate_diff_applies_append_delta_to_base_value() {
        let records = vec![TraceRecord {
            op: TraceOp::Append,
            name: "PATH".to_string(),
            old: Some("/start-shell/bin".to_string()),
            operand: Some(":/opt/tool/bin".to_string()),
        }];
        let base = env(&[("PATH", "/attach-shell/bin")]);

        assert_eq!(
            generate_diff_from_trace(&records, &base),
            diff(&[("PATH", "/attach-shell/bin:/opt/tool/bin")], &[])
        );
    }

    #[test]
    fn generate_diff_uses_empty_base_when_target_lacks_value() {
        // The attaching shell has no CPATH at all: the base is empty. The
        // recorded old value belongs to the *start* shell's stack and must
        // not leak into a target that never had the variable.
        let records = vec![TraceRecord {
            op: TraceOp::Prepend,
            name: "CPATH".to_string(),
            old: Some("/first-stack/include".to_string()),
            operand: Some("/shared/include:".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[("CPATH", "/shared/include:")], &[])
        );
    }

    #[test]
    fn generate_diff_accumulates_successive_growth() {
        let records = vec![
            TraceRecord {
                op: TraceOp::Prepend,
                name: "PATH".to_string(),
                old: Some("/base".to_string()),
                operand: Some("/p1:".to_string()),
            },
            TraceRecord {
                op: TraceOp::Append,
                name: "PATH".to_string(),
                old: Some("/p1:/base".to_string()),
                operand: Some(":/a1".to_string()),
            },
            TraceRecord {
                op: TraceOp::Prepend,
                name: "PATH".to_string(),
                old: Some("/p1:/base:/a1".to_string()),
                operand: Some("/p2:".to_string()),
            },
        ];
        let base = env(&[("PATH", "/mine")]);

        assert_eq!(
            generate_diff_from_trace(&records, &base),
            diff(&[("PATH", "/p2:/p1:/mine:/a1")], &[])
        );
    }

    #[test]
    fn generate_diff_appends_to_a_value_the_trace_itself_set() {
        let records = vec![
            TraceRecord {
                op: TraceOp::Set,
                name: "VAR".to_string(),
                old: None,
                operand: Some("value".to_string()),
            },
            TraceRecord {
                op: TraceOp::Append,
                name: "VAR".to_string(),
                old: Some("value".to_string()),
                operand: Some("-suffix".to_string()),
            },
        ];

        // The base value is irrelevant: the trace assigned an absolute value
        // before growing it.
        assert_eq!(
            generate_diff_from_trace(&records, &env(&[("VAR", "unrelated")])),
            diff(&[("VAR", "value-suffix")], &[])
        );
    }

    #[test]
    fn generate_diff_set_then_unset_nets_to_deletion() {
        let records = vec![
            TraceRecord {
                op: TraceOp::Set,
                name: "VAR".to_string(),
                old: None,
                operand: Some("value".to_string()),
            },
            TraceRecord {
                op: TraceOp::Unset,
                name: "VAR".to_string(),
                old: Some("value".to_string()),
                operand: None,
            },
        ];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[], &["VAR"])
        );
    }

    #[test]
    fn generate_diff_set_if_absent_keeps_target_value_when_present() {
        // A same-value assignment without declared intent (e.g. envrc's
        // `export VAR="${VAR:-default}"` hitting a flox-injected default)
        // defers to the target: a shell that holds its own value keeps it
        // rather than being clobbered with the start context's value.
        let records = vec![TraceRecord {
            op: TraceOp::SetIfAbsent,
            name: "SSL_CERT_FILE".to_string(),
            old: Some("/flox-injected/ca-bundle.crt".to_string()),
            operand: Some("/flox-injected/ca-bundle.crt".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &env(&[("SSL_CERT_FILE", "/my-own/certs.pem")])),
            diff(&[], &[])
        );
    }

    #[test]
    fn generate_diff_set_if_absent_sets_absent_target_value() {
        // The same-value assignment was still an assignment: a shell
        // attaching without the variable sees it defined.
        let records = vec![TraceRecord {
            op: TraceOp::SetIfAbsent,
            name: "FOO".to_string(),
            old: Some("defined".to_string()),
            operand: Some("defined".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[("FOO", "defined")], &[])
        );
    }

    #[test]
    fn generate_diff_reset_overwrites_unconditionally() {
        // Declared reset intent (BASH_ENVTRACE_RESET covered the variable
        // at mutation time, as the manifest `[vars]` application does) is
        // authoritative: the target's differing value is overwritten.
        let records = vec![TraceRecord {
            op: TraceOp::Reset,
            name: "FOO".to_string(),
            old: Some("defined".to_string()),
            operand: Some("defined".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &env(&[("FOO", "something-else")])),
            diff(&[("FOO", "defined")], &[])
        );
    }

    #[test]
    fn generate_diff_ignores_tempenv() {
        let records = vec![TraceRecord {
            op: TraceOp::Tempenv,
            name: "VAR".to_string(),
            old: None,
            operand: Some("value".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[], &[])
        );
    }

    #[test]
    fn generate_diff_ignores_bash_bookkeeping_vars() {
        // When the invoking environment exports `_`, bash's per-command
        // updates of it are environment-visible and land in the trace.
        let records = vec![TraceRecord {
            op: TraceOp::Updated,
            name: "_".to_string(),
            old: Some("/usr/bin/jq".to_string()),
            operand: Some("/usr/bin/true".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[], &[])
        );
    }

    #[test]
    fn generate_diff_ignores_exported_functions() {
        // A hook's `export -f` names the variable `BASH_FUNC_foo%%`, which
        // the shells an activation is replayed into cannot assign.
        let records = vec![TraceRecord {
            op: TraceOp::Set,
            name: "BASH_FUNC_foo%%".to_string(),
            old: None,
            operand: Some("() { true; }".to_string()),
        }];

        assert_eq!(
            generate_diff_from_trace(&records, &HashMap::new()),
            diff(&[], &[])
        );
    }

    #[test]
    fn from_state_dir_replays_the_trace_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(ENV_TRACE_LOG),
            record(["1.0", "set", "01", "TRACED", "@", ":value"]) + "\n",
        )
        .unwrap();

        let trace = EnvTrace::from_state_dir(dir.path()).unwrap();

        assert_eq!(
            trace.generate_diff(&HashMap::new()),
            diff(&[("TRACED", "value")], &[])
        );
    }

    #[test]
    fn from_state_dir_errors_without_a_trace_log() {
        let dir = tempfile::tempdir().unwrap();

        assert!(EnvTrace::from_state_dir(dir.path()).is_err());
    }

    #[test]
    fn from_state_dir_errors_on_empty_trace_log() {
        // The activate script creates the log before the tracer opens it, so
        // an empty file means the tracer failed.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ENV_TRACE_LOG), "").unwrap();

        assert!(EnvTrace::from_state_dir(dir.path()).is_err());
    }

    #[test]
    fn from_state_dir_errors_on_malformed_trace_log() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ENV_TRACE_LOG), "not a trace record\n").unwrap();

        assert!(EnvTrace::from_state_dir(dir.path()).is_err());
    }
}
