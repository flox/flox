mod flake;

use std::path::Path;

pub use flake::*;

struct Flake {}

impl Flake {
    fn determine_default_flake(path_str: String) {
        let _path = Path::new(&path_str);
    }
}
