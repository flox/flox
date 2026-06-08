//! Ordering of `[vars]` entries for the activation `envrc`.
//!
//! `[vars]` are rendered to the activation script as a sequence of
//! `export NAME="VALUE"` lines that bash sources in order. Because bash
//! expands each value as it is sourced, a value that references another
//! `[vars]` entry only resolves when the referenced entry has already been
//! exported. The entries are stored in a [`BTreeMap`], so without
//! reordering the emission order is alphabetical — whether one variable can
//! reference another then depends on the accident of their names.
//!
//! [`render_order`] fixes that by emitting each variable after the `[vars]`
//! entries it references. It does not rewrite values; bash still performs the
//! expansion. The parsing here serves only to order the entries and to reject
//! reference cycles, which bash would otherwise surface as an opaque
//! `unbound variable` failure at activation time.
//!
//! Reference detection is deliberately conservative. An edge is added only
//! when a value clearly references another entry's exact name; anything
//! ambiguous adds no edge, so a value that already worked cannot be turned
//! into a spurious cycle error. The worst case of a missed reference is the
//! pre-existing behavior — the reference fails to resolve at activation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A reference cycle among `[vars]` entries, as the chain of names that
/// forms the loop (e.g. `["foo", "bar", "foo"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VarsCycle(pub Vec<String>);

impl fmt::Display for VarsCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join(" → "))
    }
}

impl std::error::Error for VarsCycle {}

/// Order the `[vars]` keys so that each variable is emitted after every
/// `[vars]` entry it references.
///
/// This reordering is stable with respect to independent vars: a variable keeps
/// its alphabetical position unless a reference forces it later, so a manifest
/// with no cross-references renders in the same order as before. Returns
/// [`VarsCycle`] if the references form a cycle.
pub(crate) fn render_order(vars: &BTreeMap<String, String>) -> Result<Vec<String>, VarsCycle> {
    let keys: BTreeSet<String> = vars.keys().cloned().collect();

    // For each variable, the set of other `[vars]` keys it references. These
    // are its dependencies: it must be emitted after all of them.
    let dependencies: BTreeMap<String, BTreeSet<String>> = vars
        .iter()
        .map(|(name, value)| (name.clone(), referenced_keys(value, &keys, name)))
        .collect();

    // Reverse edges and in-degrees for Kahn's algorithm. We use these to count down dependencies
    // and find entries with no remaining dependencies.
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    for (name, referenced_names) in &dependencies {
        indegree.insert(name.clone(), referenced_names.len());
        for reference in referenced_names {
            dependents
                .entry(reference.clone())
                .or_default()
                .insert(name.clone());
        }
    }

    // Kahn's algorithm: repeatedly emit entries with no remaining dependencies, and
    // decrement the in-degrees of their dependents. If there are no entries with
    // zero in-degree but some entries remain, then the remaining entries form a cycle.
    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter(|&(_, &count)| count == 0)
        .map(|(name, _)| name.clone())
        .collect();
    let mut order = Vec::with_capacity(dependencies.len());
    while let Some(name) = ready.pop_first() {
        for dependent in dependents.get(&name).into_iter().flatten() {
            let count = indegree.get_mut(dependent).expect("dependent is a key");
            *count -= 1;
            if *count == 0 {
                ready.insert(dependent.clone());
            }
        }
        order.push(name);
    }

    if order.len() != dependencies.len() {
        return Err(VarsCycle(find_cycle(&dependencies, &order)));
    }
    Ok(order)
}

/// The `[vars]` keys referenced by `value`, excluding `self_name`.
///
/// Mirrors bash expansion inside `export NAME="<value>"`: `$name` and
/// `${name...}` are expansions, and a `$` preceded by a backslash is a literal
/// dollar. Only names that are exactly another `[vars]` key are returned;
/// environment variables, command substitutions, and undefined names impose no
/// ordering constraint and are ignored. A variable's reference to its own name
/// reads the ambient value (the `PATH = "${PATH}:/x"` idiom) and is not a
/// dependency.
fn referenced_keys(value: &str, keys: &BTreeSet<String>, self_name: &str) -> BTreeSet<String> {
    let bytes = value.as_bytes();
    let mut deps = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            // A backslash escapes the next byte, so we simply skip ahead.
            b'\\' => i += 2,
            // A $ is either $FOO or ${FOO...}. In either case we look for the following name and
            // add an edge if it matches a key. If the { is unterminated, we ignore the reference
            // and assume it will create a syntax error at activation time. Notably, we search for
            // a valid name immediately, but accept a closing curly brace anywhere. This allows us
            // to detect references in common parameter expansion forms like `${FOO:-default}`
            // without needing to parse the operator or worry about nested expansions.
            b'$' => {
                let mut j = i + 1;
                let braced = j < bytes.len() && bytes[j] == b'{';
                if braced {
                    j += 1;
                }
                let name_start = j;
                while j < bytes.len() && is_name_byte(bytes[j], j == name_start) {
                    j += 1;
                }
                let closed = !braced || value[j..].contains('}');
                if j > name_start && closed {
                    let name = &value[name_start..j];
                    if name != self_name && keys.contains(name) {
                        deps.insert(name.to_string());
                    }
                }
                // `j` is past the `$`, so scanning resumes and always advances.
                i = j;
            },
            _ => i += 1,
        }
    }
    deps
}

/// Whether `b` may appear in a shell variable name. The first byte may not be
/// a digit, which excludes positional parameters like `${1}`.
fn is_name_byte(b: u8, first: bool) -> bool {
    match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => true,
        b'0'..=b'9' => !first,
        _ => false,
    }
}

/// Recover one reference cycle from the entries left unemitted by the
/// topological sort, as the chain of names that closes the loop.
fn find_cycle(deps: &BTreeMap<String, BTreeSet<String>>, order: &[String]) -> Vec<String> {
    let emitted: BTreeSet<&String> = order.iter().collect();
    let remaining: BTreeSet<&String> = deps.keys().filter(|k| !emitted.contains(*k)).collect();

    let Some(start) = remaining.first().copied() else {
        return Vec::new();
    };

    // Every unemitted entry references at least one other unemitted entry, so
    // following those references eventually revisits a name. That repeat closes
    // the cycle.
    let mut path = vec![start.clone()];
    loop {
        let next = deps[path.last().expect("path is never empty")]
            .iter()
            .find(|reference| remaining.contains(reference))
            .expect("an unemitted entry references another unemitted entry")
            .clone();
        if let Some(start_idx) = path.iter().position(|name| *name == next) {
            path.push(next);
            return path.split_off(start_idx);
        }
        path.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn refs(value: &str, keys: &[&str], self_name: &str) -> Vec<String> {
        let keys: BTreeSet<String> = keys.iter().map(|k| k.to_string()).collect();
        referenced_keys(value, &keys, self_name)
            .into_iter()
            .collect()
    }

    #[test]
    fn no_references_preserves_alphabetical_order() {
        let order = render_order(&vars(&[("b", "2"), ("a", "1"), ("c", "3")])).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn dependency_emitted_before_dependent() {
        let order = render_order(&vars(&[("a", "${z}"), ("z", "1")])).unwrap();
        assert_eq!(order, vec!["z", "a"]);
    }

    #[test]
    fn reference_to_alphabetically_later_var_is_reordered() {
        // The original bug: `aaa` references `zzz`, which sorts after it.
        let order = render_order(&vars(&[("aaa", "${zzz}"), ("zzz", "v")])).unwrap();
        assert_eq!(order, vec!["zzz", "aaa"]);
    }

    #[test]
    fn only_forced_entries_move() {
        // Constraint is z < a; `m` is unconstrained and keeps its slot.
        let order = render_order(&vars(&[("a", "${z}"), ("m", "x"), ("z", "1")])).unwrap();
        assert_eq!(order, vec!["m", "z", "a"]);
    }

    #[test]
    fn self_reference_is_not_a_cycle() {
        let order = render_order(&vars(&[("PATH", "${PATH}:/x")])).unwrap();
        assert_eq!(order, vec!["PATH"]);
    }

    #[test]
    fn self_reference_with_additional_dependency() {
        let order = render_order(&vars(&[("PATH", "${PATH}:${bin}"), ("bin", "/b")])).unwrap();
        assert_eq!(order, vec!["bin", "PATH"]);
    }

    #[test]
    fn bare_dollar_reference_creates_edge() {
        let order = render_order(&vars(&[("a", "$z"), ("z", "1")])).unwrap();
        assert_eq!(order, vec!["z", "a"]);
    }

    #[test]
    fn environment_reference_adds_no_edge() {
        // `USER` is not a `[vars]` key; bash resolves it from the environment.
        let order = render_order(&vars(&[("greeting", "${USER}")])).unwrap();
        assert_eq!(order, vec!["greeting"]);
        assert_eq!(
            refs("${USER}", &["greeting"], "greeting"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn command_substitution_inner_reference_creates_edge() {
        let order = render_order(&vars(&[("a", "$(echo ${b})"), ("b", "1")])).unwrap();
        assert_eq!(order, vec!["b", "a"]);
    }

    #[test]
    fn nested_reference_creates_edge() {
        let order = render_order(&vars(&[("a", "${x:-${y}}"), ("x", "1"), ("y", "2")])).unwrap();
        assert_eq!(order, vec!["x", "y", "a"]);
    }

    #[test]
    fn escaped_dollar_is_not_a_reference() {
        assert_eq!(refs(r"\${b}", &["b"], "a"), Vec::<String>::new());
        let order = render_order(&vars(&[("a", r"\${b}"), ("b", "1")])).unwrap();
        assert_eq!(order, vec!["a", "b"]);
    }

    #[test]
    fn reference_matches_whole_name_only() {
        // `${ab}` references `ab`, not the shorter key `a`.
        assert_eq!(refs("${ab}", &["a", "ab"], "x"), vec!["ab".to_string()]);
        assert_eq!(refs("$ab", &["a", "ab"], "x"), vec!["ab".to_string()]);
    }

    #[test]
    fn parameter_expansion_with_operator_creates_edge() {
        assert_eq!(refs("${b:-default}", &["b"], "a"), vec!["b".to_string()]);
    }

    #[test]
    fn positional_parameter_is_not_a_reference() {
        assert_eq!(refs("${1}", &["a"], "a"), Vec::<String>::new());
    }

    #[test]
    fn unclosed_brace_is_not_a_reference() {
        // `${foo` is a bash syntax error, not a reference; adding no edge keeps
        // a malformed value from reordering anything.
        assert_eq!(refs("${foo", &["foo"], "bar"), Vec::<String>::new());
        let order = render_order(&vars(&[("bar", "${foo"), ("foo", "x")])).unwrap();
        assert_eq!(order, vec!["bar", "foo"]);
    }

    #[test]
    fn unclosed_brace_does_not_create_false_cycle() {
        // `bar`'s value is malformed and imposes no ordering, so `foo`'s
        // reference to `bar` is the only edge and there is no cycle.
        let order = render_order(&vars(&[("bar", "${foo"), ("foo", "${bar}")])).unwrap();
        assert_eq!(order, vec!["bar", "foo"]);
    }

    #[test]
    fn mutual_cycle_is_detected() {
        let err = render_order(&vars(&[("foo", "${bar}"), ("bar", "${foo}")])).unwrap_err();
        assert_eq!(
            err,
            VarsCycle(vec![
                "bar".to_string(),
                "foo".to_string(),
                "bar".to_string(),
            ])
        );
        assert_eq!(err.to_string(), "bar → foo → bar");
    }

    #[test]
    fn three_way_cycle_is_detected() {
        let err = render_order(&vars(&[("a", "${b}"), ("b", "${c}"), ("c", "${a}")])).unwrap_err();
        assert_eq!(
            err,
            VarsCycle(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "a".to_string(),
            ])
        );
    }

    #[test]
    fn self_reference_alone_does_not_form_cycle() {
        assert!(render_order(&vars(&[("a", "${a}")])).is_ok());
    }

    #[test]
    fn empty_vars_yield_empty_order() {
        assert_eq!(
            render_order(&BTreeMap::new()).unwrap(),
            Vec::<String>::new()
        );
    }
}
