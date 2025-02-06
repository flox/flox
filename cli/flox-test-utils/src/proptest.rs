use std::collections::BTreeMap;

use proptest::prelude::*;

pub fn chrono_strat() -> impl Strategy<Value = chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;

    let start = chrono::Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap();
    let end = chrono::Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap();

    (start.timestamp()..end.timestamp())
        .prop_map(|timestamp| chrono::Utc.timestamp_opt(timestamp, 0).unwrap())
}

/// Produces strings that only contain alphanumeric characters.
///
/// This is handy when you want to generate valid TOML keys without worrying about quoting
/// or escaping.
pub fn alphanum_string(max_size: usize) -> impl Strategy<Value = String> {
    let ranges = vec!['a'..='z', 'A'..='Z', '0'..='9'];
    prop::collection::vec(
        proptest::char::ranges(std::borrow::Cow::Owned(ranges)),
        1..max_size,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// Produces maps whose keys are strings that only contain alphanumeric characters.
pub fn btree_map_alphanum_keys<T: proptest::arbitrary::Arbitrary>(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = BTreeMap<String, T>> {
    prop::collection::btree_map(alphanum_string(key_max_size), any::<T>(), 0..max_keys)
}

/// Produces maps whose keys are strings that only contain alphanumeric
/// characters and whose values are empty BTreeMaps
pub fn empty_btree_map_alphanum_keys(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = BTreeMap<String, BTreeMap<(), ()>>> {
    use prop::collection::btree_map;

    prop::collection::btree_map(
        alphanum_string(key_max_size),
        btree_map(any::<()>(), any::<()>(), 0),
        0..max_keys,
    )
}
