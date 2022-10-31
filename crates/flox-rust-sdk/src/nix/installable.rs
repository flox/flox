#[derive(Debug, Clone)]
pub struct Installable {
    flakeref: String,
    attr_path: String,
}

impl Installable {
    pub fn to_nix(&self) -> String {
        format!("{}#{}", self.flakeref, self.attr_path)
    }
}

impl From<String> for Installable {
    fn from(input: String) -> Self {
        let mut split = input.splitn(2, '#');

        match (split.next(), split.next()) {
            (Some(flakeref), Some(attr_path)) => Installable {
                flakeref: flakeref.to_owned(),
                attr_path: attr_path.to_owned(),
            },
            (Some(attr_path), None) => Installable {
                flakeref: ".".to_owned(),
                attr_path: attr_path.to_owned(),
            },
            _ => unreachable!(),
        }
    }
}
