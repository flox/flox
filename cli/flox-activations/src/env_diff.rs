use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

pub struct EnvDiff {
    pub additions: HashMap<String, String>,
    pub deletions: Vec<String>,
}

impl EnvDiff {
    pub fn new() -> Self {
        Self {
            additions: HashMap::new(),
            deletions: Vec::new(),
        }
    }

    /// Load an EnvDiff from start.env.json and end.env.json files in activation_state_dir
    pub fn from_files(activation_state_dir: impl AsRef<Path>) -> Result<EnvDiff> {
        let start_json = activation_state_dir.as_ref().join("start.env.json");
        let end_json = activation_state_dir.as_ref().join("end.env.json");

        let start_env = parse_env_json(start_json)?;
        let end_env = parse_env_json(end_json)?;

        Ok(from_parsed_files(&start_env, &end_env))
    }
}

impl Default for EnvDiff {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a JSON environment file (output of `jq -nS env`) into a HashMap.
///
/// The JSON file should be an object with environment variable names as keys
/// and their values as string values.
fn parse_env_json(path: impl AsRef<Path>) -> Result<HashMap<String, String>> {
    let contents = std::fs::read_to_string(path.as_ref())?;
    Ok(serde_json::from_str(&contents)?)
}

fn from_parsed_files(
    start_env: &HashMap<String, String>,
    end_env: &HashMap<String, String>,
) -> EnvDiff {
    let mut env_diff = EnvDiff::new();

    for key in start_env.keys() {
        if !end_env.contains_key(key) {
            env_diff.deletions.push(key.clone());
        }
    }
    for (key, value) in end_env {
        if start_env.get(key) != Some(value) {
            env_diff.additions.insert(key.clone(), value.clone());
        }
    }
    env_diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_parsed_files_insertion() {
        let start_env = HashMap::from([("VAR1".to_string(), "value1".to_string())]);
        let end_env = HashMap::from([
            ("VAR1".to_string(), "value1".to_string()),
            ("VAR2".to_string(), "value2".to_string()),
        ]);

        let diff = from_parsed_files(&start_env, &end_env);

        assert_eq!(
            diff.additions,
            HashMap::from([("VAR2".to_string(), "value2".to_string()),])
        );
        assert!(diff.deletions.is_empty());
    }

    #[test]
    fn test_from_parsed_files_deletion() {
        let start_env = HashMap::from([("VAR1".to_string(), "value1".to_string())]);
        let end_env = HashMap::new();

        let diff = from_parsed_files(&start_env, &end_env);

        assert!(diff.additions.is_empty());
        assert_eq!(diff.deletions, vec!["VAR1".to_string()]);
    }

    #[test]
    fn test_from_parsed_files_unchanged() {
        let start_env = HashMap::from([("VAR1".to_string(), "value1".to_string())]);

        let diff = from_parsed_files(&start_env, &start_env);

        assert!(diff.additions.is_empty());
        assert!(diff.deletions.is_empty());
    }

    #[test]
    fn test_from_parsed_files_changed_value() {
        let start_env = HashMap::from([("VAR1".to_string(), "old_value".to_string())]);
        let end_env = HashMap::from([("VAR1".to_string(), "new_value".to_string())]);

        let diff = from_parsed_files(&start_env, &end_env);

        assert_eq!(
            diff.additions,
            HashMap::from([("VAR1".to_string(), "new_value".to_string())])
        );
        assert!(diff.deletions.is_empty());
    }
}
