pub mod common;
pub mod latest;
pub mod v1;
pub mod v1_10_0;

/// An interface codifying how to access types that are just semantic wrappers
/// around inner types. This impl may be generated with a macro.
pub trait Inner {
    type Inner;

    fn inner(&self) -> &Self::Inner;
    fn inner_mut(&mut self) -> &mut Self::Inner;
    fn into_inner(self) -> Self::Inner;
}

/// A macro that generates a `Inner` impl.
macro_rules! impl_into_inner {
    ($wrapper:ty, $inner_type:ty) => {
        impl crate::parsed::Inner for $wrapper {
            type Inner = $inner_type;

            fn inner(&self) -> &Self::Inner {
                &self.0
            }

            fn inner_mut(&mut self) -> &mut Self::Inner {
                &mut self.0
            }

            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

pub(crate) use impl_into_inner;

/// An interface for the type of function that serde's skip_serializing_if
/// method takes.
pub(crate) trait SkipSerializing {
    fn skip_serializing(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use indoc::indoc;
    use serde_json::{Number, Value};

    use crate::Manifest;
    use crate::parsed::common::KnownSchemaVersion;
    use crate::test_helpers::with_schema;

    #[test]
    fn parses_all_schema_versions() {
        let body = indoc! {"
            [install]
            hello.pkg-path = \"hello\"

            [hook]
            on-activate = '''
                echo foo
            '''

            [profile]
            bash = '''
                echo \"use fish instead\"
            '''

            [options]
            allow.unfree = true
        "};
        for schema in KnownSchemaVersion::iter() {
            let manifest = with_schema(schema, body);
            Manifest::parse_toml_untyped(manifest).unwrap();
        }
    }

    #[test]
    fn deserializes_null_fields() {
        for schema in KnownSchemaVersion::iter() {
            let schema_value = match schema {
                KnownSchemaVersion::V1 => {
                    Value::Number(Number::from_str(schema.to_string().as_str()).unwrap())
                },
                KnownSchemaVersion::V1_10_0 => Value::String(schema.to_string()),
            };
            let json_value = serde_json::json!({
                schema.key_name(): schema_value,
                "hook": {
                    "on-activate": null
                },
                "profile": {
                    "common": null,
                    "bash": null,
                    "fish": null,
                    "zsh": null,
                    "tcsh": null
                },
                "options": {}
            });

            let json = serde_json::ser::to_string_pretty(&json_value).unwrap();
            Manifest::parse_json(json).unwrap();
        }
    }
}
