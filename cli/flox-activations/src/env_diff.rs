use std::collections::HashMap;

/// Wrapper around environment variable additions and deletions
///
/// This is a minimal struct that is re-used in various other places to pass
/// around environment variable changes
#[derive(Debug, Clone, Default)]
pub struct EnvDiff {
    pub additions: HashMap<String, String>,
    pub deletions: Vec<String>,
}

impl EnvDiff {
    pub fn from_parts(additions: HashMap<String, String>, deletions: Vec<String>) -> Self {
        Self {
            additions,
            deletions,
        }
    }
}
