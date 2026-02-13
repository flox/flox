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
    use indoc::indoc;

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
            Manifest::parse_untyped(manifest).unwrap();
        }
    }
}
