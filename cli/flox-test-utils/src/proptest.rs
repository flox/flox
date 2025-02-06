use std::collections::{BTreeMap, BTreeSet};

use prop::option;
use proptest::collection::{btree_map, btree_set as prop_btree_set, vec as prop_vec};
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
        1..=max_size,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// Produces strings that only contain alphanumeric characters.
///
/// This is handy when you want to generate valid TOML keys without worrying about quoting
/// or escaping.
pub fn alphanum_and_whitespace_string(max_size: usize) -> impl Strategy<Value = String> {
    let ranges = vec![
        'a'..='z',
        'A'..='Z',
        '0'..='9',
        ' '..=' ',
        char::from(9)..=char::from(10),  // tab and lf
        char::from(13)..=char::from(13), // cr
    ];
    prop::collection::vec(
        proptest::char::ranges(std::borrow::Cow::Owned(ranges)),
        1..=max_size,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// Produces `Option<String>` instances with limited sizes for performance
/// reasons.
pub fn optional_string(string_max_size: usize) -> impl Strategy<Value = Option<String>> {
    option::of(alphanum_string(string_max_size))
}

/// Produces maps whose keys are strings that only contain alphanumeric characters.
pub fn btree_map_strategy<T: Arbitrary>(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = BTreeMap<String, T>> {
    btree_map(alphanum_string(key_max_size), any::<T>(), 0..max_keys)
}

/// Produces optional maps with limited sizes for performance reasons
pub fn optional_btree_map<T: Arbitrary>(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = Option<BTreeMap<String, T>>> {
    option::of(btree_map_strategy(key_max_size, max_keys))
}

/// Produces maps whose keys are strings that only contain alphanumeric characters.
pub fn btree_set(key_max_size: usize, max_keys: usize) -> impl Strategy<Value = BTreeSet<String>> {
    prop_btree_set(alphanum_string(key_max_size), 0..max_keys)
}

/// Produces optional sets with limited sizes for performance reasons
/// Produces maps whose keys are strings that only contain alphanumeric characters.
pub fn optional_btree_set(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = Option<BTreeSet<String>>> {
    option::of(btree_set(key_max_size, max_keys))
}

/// Produces maps whose keys are strings that only contain alphanumeric
/// characters and whose values are empty BTreeMaps
pub fn empty_btree_map_alphanum_keys(
    key_max_size: usize,
    max_keys: usize,
) -> impl Strategy<Value = BTreeMap<String, BTreeMap<(), ()>>> {
    btree_map(
        alphanum_string(key_max_size),
        btree_map(any::<()>(), any::<()>(), 0),
        0..max_keys,
    )
}

/// Produces `Vec<String>` instances with limited sizes for performance reasons.
pub fn vec_of_strings(
    string_max_size: usize,
    max_elements: usize,
) -> impl Strategy<Value = Vec<String>> {
    prop_vec(alphanum_string(string_max_size), 0..=max_elements)
}

/// Produces `Option<Vec<String>>` instances with limited sizes for performance
/// reasons
pub fn optional_vec_of_strings(
    string_max_size: usize,
    max_elements: usize,
) -> impl Strategy<Value = Option<Vec<String>>> {
    option::of(prop_vec(alphanum_string(string_max_size), 1..=max_elements))
}

/// A container for randomly generated maps with overlapping keys
#[derive(Debug, Clone)]
pub struct OverlappingMaps<T> {
    pub unique_keys_map1: Vec<String>,
    pub unique_keys_map2: Vec<String>,
    pub duplicate_keys: Vec<String>,
    pub map1: BTreeMap<String, T>,
    pub map2: BTreeMap<String, T>,
}

/// Produces two maps whose keys overlap.
///
/// The return values are the two maps whose keys overlap and the overlapping keys.
/// Note that this returns a `BoxedStrategy` purely for the sake of not killing
/// `rust-analyzer` with the horendous type that would otherwise be generated as
/// the return type for this function.
pub fn btree_maps_overlapping_keys<T: Arbitrary>(
    key_max_size: usize,
    max_keys: usize,
) -> BoxedStrategy<OverlappingMaps<T>> {
    (0..max_keys, 0..max_keys, 0..max_keys)
        .prop_flat_map(move |(n_keys_map1, n_keys_map2, n_dupes)| {
            (
                // Keys unique to map 1
                prop_vec(alphanum_string(key_max_size), n_keys_map1..=n_keys_map1),
                // Keys unique to map 2
                prop_vec(alphanum_string(key_max_size), n_keys_map2..=n_keys_map2),
                // Keys duplicated between the maps
                prop_vec(alphanum_string(key_max_size), n_dupes..=n_dupes),
                // Values for the unique keys in map 1
                prop_vec(any::<T>(), n_keys_map1..=n_keys_map1),
                // Values for the unique keys in map 2
                prop_vec(any::<T>(), n_keys_map2..=n_keys_map2),
                // Values for the duplicate keys in map 1
                prop_vec(any::<T>(), n_dupes..=n_dupes),
                // Values for the duplicate keys in map 2
                prop_vec(any::<T>(), n_dupes..=n_dupes),
            )
        })
        .prop_filter(
            "ensure no pre-existing overlap",
            |(
                keys_map1,
                keys_map2,
                keys_dup,
                _vals_map1,
                _vals_map2,
                _vals_dup_map1,
                _vals_dup_map2,
            )| {
                // Ensure that there's no pre-existing overlap with the duplicate keys
                for key in keys_map1.iter() {
                    if keys_dup.contains(key) {
                        return false;
                    }
                    if keys_map2.contains(key) {
                        return false;
                    }
                    // It's actually fine if there's overlap between keys_dup and map 2
                    // so we don't check that case
                }
                true
            },
        )
        .prop_map(
            |(
                keys_map1,
                keys_map2,
                keys_dup,
                vals_map1,
                vals_map2,
                vals_dup_map1,
                vals_dup_map2,
            )| {
                let mut map1 = BTreeMap::new();
                let mut map2 = BTreeMap::new();
                for (k, v) in keys_map1.clone().into_iter().zip(vals_map1.into_iter()) {
                    map1.insert(k, v);
                }
                for (k, v) in keys_dup.clone().into_iter().zip(vals_dup_map1.into_iter()) {
                    map1.insert(k, v);
                }
                for (k, v) in keys_map2.clone().into_iter().zip(vals_map2.into_iter()) {
                    map2.insert(k, v);
                }
                for (k, v) in keys_dup.clone().into_iter().zip(vals_dup_map2.into_iter()) {
                    map2.insert(k, v);
                }
                OverlappingMaps {
                    unique_keys_map1: keys_map1,
                    unique_keys_map2: keys_map2,
                    duplicate_keys: keys_dup,
                    map1,
                    map2,
                }
            },
        )
        .boxed()
}
