use std::cmp::Ordering;
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

use regex::Regex;

#[derive(Debug, PartialEq, Eq)]
pub enum PreReleaseName {
    Alpha,
    Beta,
    RC,
}

impl PartialOrd for PreReleaseName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreReleaseName {
    fn cmp(&self, other: &Self) -> Ordering {
        use PreReleaseName::*;
        match (self, other) {
            (Alpha, Alpha) => Ordering::Equal,
            (Beta, Beta) => Ordering::Equal,
            (RC, RC) => Ordering::Equal,
            (Alpha, _) => Ordering::Less,
            (Beta, RC) => Ordering::Less,
            (RC, _) => Ordering::Greater,
            (Beta, Alpha) => Ordering::Greater,
        }
    }
}

#[derive(Debug)]
pub enum PreReleaseNameParseError {
    InvalidPreRelease,
}

impl FromStr for PreReleaseName {
    type Err = PreReleaseNameParseError;

    fn from_str(pre_name_str: &str) -> Result<Self, Self::Err> {
        match pre_name_str {
            "alpha" => Ok(PreReleaseName::Alpha),
            "beta" => Ok(PreReleaseName::Beta),
            "rc" => Ok(PreReleaseName::RC),
            _ => Err(PreReleaseNameParseError::InvalidPreRelease),
        }
    }
}

impl fmt::Display for PreReleaseName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreReleaseName::Alpha => write!(f, "alpha"),
            PreReleaseName::Beta => write!(f, "beta"),
            PreReleaseName::RC => write!(f, "rc"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct FloxVersion {
    major: u32,
    minor: u32,
    patch: u32,
    pre_name: Option<PreReleaseName>,
    pre_number: Option<u32>,
    num_of_commits: Option<u32>,
    commit_vcs: Option<char>,
    commit_sha: Option<String>,
}

#[derive(Debug)]
pub enum VersionParseError {
    InvalidFormat,
    InvalidNumber(ParseIntError),
}

impl From<ParseIntError> for VersionParseError {
    fn from(err: ParseIntError) -> Self {
        VersionParseError::InvalidNumber(err)
    }
}

impl FromStr for FloxVersion {
    type Err = VersionParseError;

    fn from_str(version_str: &str) -> Result<Self, Self::Err> {
        // Define the regex pattern
        let re = Regex::new(r"(?x)
            ^(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)        # Match major.minor.patch
            (?:-(?P<pre>(?P<pre_name>[a-zA-Z]+)\.(?P<pre_number>\d+)))? # Optionally match pre-release name and number (e.g., rc.1)
            (?:-(?P<num_of_commits>\d+))?                          # Optionally match number of commits
            (?:-(?P<commit_vcs>[a-z])(?P<commit_sha>[a-f0-9]+))?   # Optionally match VCS and SHA
        $").unwrap(); // Unwrap is safe here because the regex is a constant

        // Apply the regex to the version string
        if let Some(captures) = re.captures(version_str) {
            let pre_name = captures
                .name("pre_name")
                .map(|s| s.as_str().parse().unwrap());
            let pre_number = captures
                .name("pre_number")
                .map(|s| s.as_str().parse().unwrap());
            let num_of_commits = captures
                .name("num_of_commits")
                .map(|s| s.as_str().parse().unwrap());
            let commit_vcs = captures
                .name("commit_vcs")
                .map(|s| s.as_str().chars().next().unwrap());
            let commit_sha = captures.name("commit_sha").map(|s| s.as_str().to_string());

            // When there is pre release commit fields shouldn't be there
            if (pre_name.is_some() || pre_number.is_some())
                && (num_of_commits.is_some() || commit_vcs.is_some() || commit_sha.is_some())
            {
                return Err(VersionParseError::InvalidFormat);
            }

            // There can never by number of commits without the commit sha
            if num_of_commits.is_some() && (commit_vcs.is_none() || commit_sha.is_none()) {
                return Err(VersionParseError::InvalidFormat);
            }

            Ok(FloxVersion {
                major: captures["major"].parse()?,
                minor: captures["minor"].parse()?,
                patch: captures["patch"].parse()?,
                pre_name,
                pre_number,
                num_of_commits,
                commit_vcs,
                commit_sha,
            })
        } else {
            Err(VersionParseError::InvalidFormat)
        }
    }
}

impl PartialOrd for FloxVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Compare major, minor, and patch versions
        match self.major.cmp(&other.major) {
            Ordering::Equal => (),
            ordering => return Some(ordering),
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => (),
            ordering => return Some(ordering),
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => (),
            ordering => return Some(ordering),
        }

        // Compare number of commits
        match (self.num_of_commits, other.num_of_commits) {
            (None, None) => (),
            (None, Some(_)) => return Some(Ordering::Less),
            (Some(_), None) => return Some(Ordering::Greater),
            (Some(self_commits), Some(other_commits)) => {
                return Some(self_commits.cmp(&other_commits))
            },
        }

        // Pre-release comparison
        match (&self.pre_name, &other.pre_name) {
            (None, None) => (),
            (None, Some(_)) => return Some(Ordering::Greater),
            (Some(_), None) => return Some(Ordering::Less),
            (Some(self_pre), Some(other_pre)) => match self_pre.cmp(other_pre) {
                Ordering::Equal => (),
                ordering => return Some(ordering),
            },
        }

        // Compare pre-release numbers if both have pre-release names
        match (self.pre_number, other.pre_number) {
            (None, None) => (),
            (None, Some(_)) => return Some(Ordering::Greater),
            (Some(_), None) => return Some(Ordering::Less),
            (Some(self_num), Some(other_num)) => return Some(self_num.cmp(&other_num)),
        }

        // Skip commit comparison if there are pre-release fields
        match (self.commit_vcs, other.commit_vcs) {
            (None, None) => Some(Ordering::Equal),
            _ => None,
        }
    }
}

impl fmt::Display for FloxVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Start with the mandatory major.minor.patch part
        let mut version_str = format!("{}.{}.{}", self.major, self.minor, self.patch);

        // If there is a pre-release name (e.g., "rc"), include it
        if let Some(ref pre_name) = self.pre_name {
            version_str = format!("{}-{}", version_str, pre_name);
            if let Some(pre_number) = self.pre_number {
                version_str = format!("{}.{}", version_str, pre_number);
            }
        }

        // If there is a number of commits, include it
        if let Some(num_of_commits) = self.num_of_commits {
            version_str = format!("{}-{}", version_str, num_of_commits);
        }

        // If there is a commit SHA, include it with the commit VCS prefix
        if let Some(ref commit_sha) = self.commit_sha {
            if let Some(commit_vcs) = self.commit_vcs {
                version_str = format!("{}-{}{}", version_str, commit_vcs, commit_sha);
            } else {
                version_str = format!("{}-{}", version_str, commit_sha);
            }
        }

        // Write the formatted string to the formatter
        write!(f, "{}", version_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_version() {
        let version_str = "1.2.3";
        let version: FloxVersion = version_str.parse().unwrap();
        assert_eq!(version, FloxVersion {
            major: 1,
            minor: 2,
            patch: 3,
            pre_name: None,
            pre_number: None,
            num_of_commits: None,
            commit_vcs: None,
            commit_sha: None,
        });
        assert_eq!(version.to_string(), version_str);
        assert_eq!(version.partial_cmp(&version), Some(Ordering::Equal));
    }

    #[test]
    fn test_parse_version_with_pre_release() {
        let version_str = "1.2.3-rc.1";
        let version: FloxVersion = version_str.parse().unwrap();
        assert_eq!(version, FloxVersion {
            major: 1,
            minor: 2,
            patch: 3,
            pre_name: Some(PreReleaseName::RC),
            pre_number: Some(1),
            num_of_commits: None,
            commit_vcs: None,
            commit_sha: None,
        });
        assert_eq!(version.to_string(), version_str);
        assert_eq!(version.partial_cmp(&version), Some(Ordering::Equal));
    }

    #[test]
    fn test_parse_version_with_git_describe_format() {
        let version_str = "1.2.3-21-gb91c3f1";
        let version: FloxVersion = version_str.parse().unwrap();
        assert_eq!(version, FloxVersion {
            major: 1,
            minor: 2,
            patch: 3,
            pre_name: None,
            pre_number: None,
            num_of_commits: Some(21),
            commit_vcs: Some('g'),
            commit_sha: Some("b91c3f1".to_string()),
        });
        assert_eq!(version.to_string(), version_str);
        assert_eq!(version.partial_cmp(&version), Some(Ordering::Equal));
    }

    #[test]
    fn test_parse_version_with_wrong_git_describe_format() {
        assert!(matches!(
            "1.2.3-21".parse::<FloxVersion>(),
            Err(VersionParseError::InvalidFormat),
        ));
    }

    #[test]
    fn test_parse_version_with_only_commit_sha() {
        let version_str = "1.2.3-gb91c3f1";
        let version: FloxVersion = version_str.parse().unwrap();
        assert_eq!(version, FloxVersion {
            major: 1,
            minor: 2,
            patch: 3,
            pre_name: None,
            pre_number: None,
            num_of_commits: None,
            commit_vcs: Some('g'),
            commit_sha: Some("b91c3f1".to_string()),
        });
        assert_eq!(version.to_string(), version_str);
        assert_eq!(version.partial_cmp(&version), None);
    }

    #[test]
    fn test_parse_version_with_pre_release_and_commits() {
        assert!(matches!(
            "1.2.3-rc.1-10-gb91c3f1".parse::<FloxVersion>(),
            Err(VersionParseError::InvalidFormat),
        ));
    }

    #[test]
    fn test_version_ordering() {
        // This is the order of versions from smallest to largest
        let v1: FloxVersion = "1.2.2".parse().unwrap();
        let v2: FloxVersion = "1.2.2-10-gb91c3f1".parse().unwrap();
        let v3: FloxVersion = "1.2.2-11-gb91c3f1".parse().unwrap();
        let v4: FloxVersion = "1.2.3-rc.1".parse().unwrap();
        let v5: FloxVersion = "1.2.3-rc.2".parse().unwrap();
        let v6: FloxVersion = "1.2.3".parse().unwrap();

        assert!(v1 == v1);
        assert!(v1 < v2);
        assert!(v1 < v3);
        assert!(v1 < v4);
        assert!(v1 < v5);
        assert!(v1 < v6);

        assert!(v2 > v1);
        assert!(v2 == v2);
        assert!(v2 < v3);
        assert!(v2 < v4);
        assert!(v2 < v5);
        assert!(v2 < v6);

        assert!(v3 > v1);
        assert!(v3 > v2);
        assert!(v3 == v3);
        assert!(v3 < v4);
        assert!(v3 < v5);
        assert!(v3 < v6);

        assert!(v4 > v1);
        assert!(v4 > v2);
        assert!(v4 > v3);
        assert!(v4 == v4);
        assert!(v4 < v5);
        assert!(v4 < v6);

        assert!(v5 > v1);
        assert!(v5 > v2);
        assert!(v5 > v3);
        assert!(v5 > v4);
        assert!(v5 == v5);
        assert!(v5 < v6);

        assert!(v6 > v1);
        assert!(v6 > v2);
        assert!(v6 > v3);
        assert!(v6 > v4);
        assert!(v6 > v5);
        assert!(v6 == v6);
    }

    #[test]
    fn test_version_ordering_with_flake_style_version() {
        let v1: FloxVersion = "1.2.2".parse().unwrap();
        let v2: FloxVersion = "1.2.2-gb91c3f1".parse().unwrap();

        assert!(v1 != v2);
        assert_eq!(v1.partial_cmp(&v2), None);
    }
}
