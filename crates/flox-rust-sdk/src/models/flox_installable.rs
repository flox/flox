use log::debug;
use once_cell::sync::Lazy;
use regex::Regex;
use thiserror::Error;

// Matches against strings which are likely to be flakerefs
// Such as: `github:NixOS/nixpkgs`, `.`, `../somedir`, etc
static PROBABLY_FLAKEREF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^(?:\.?\.?/|\.$|[a-z+]+:)"#).unwrap());

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct FloxInstallable {
    pub source: Option<String>,
    pub attr_path: Vec<String>,
}
#[derive(Error, Debug, PartialEq)]
pub enum ParseFloxInstallableError {
    #[error("Error parsing installable string")]
    ParseError(#[from] rnix::parser::ParseError),
    #[error("Installables must not contain interpolations")]
    ComplexString,
    #[error("Unrecognized node or token encountered")]
    Unrecognized,
}

/// Parse an installable string into the [FloxInstallable] struct, which recognizes its source and split attr path components.
/// This uses rnix to allow things like `myAttrSet."my attr with spaces"` to work correctly.
impl std::str::FromStr for FloxInstallable {
    type Err = ParseFloxInstallableError;

    fn from_str(installable_str: &str) -> Result<FloxInstallable, ParseFloxInstallableError> {
        debug!("Parsing installable: {:?}", installable_str);

        // Split `installable_str` by `#` to get the attr path selector and the source separately
        // handle the whole string as an attrpath if no `#` is found
        let (maybe_source, attr_path_str) = match installable_str.split_once('#') {
            Some((source, attr_path_str)) => (Some(source), attr_path_str),
            _ => (None, installable_str),
        };

        debug!(
            "Split installable into {:?} and {:?}",
            maybe_source, attr_path_str
        );

        // flakeref convenience cases
        if maybe_source.is_none() && PROBABLY_FLAKEREF_RE.is_match(attr_path_str) {
            return Ok(FloxInstallable {
                source: Some(attr_path_str.to_string()),
                attr_path: vec![],
            });
        }

        if attr_path_str.is_empty() {
            return Ok(FloxInstallable {
                source: maybe_source.map(String::from),
                attr_path: vec![],
            });
        }

        // Parse the provided attr path string
        let root = rnix::Root::parse(attr_path_str).ok()?;

        // Get the root expression of the attr path
        let expr = root.expr().expect("Failed to get root expression");

        fn expr_to_attr_path(
            expr: rnix::ast::Expr,
        ) -> Result<Vec<String>, ParseFloxInstallableError> {
            Ok(match expr {
                // With an attrpath like `x.y`
                rnix::ast::Expr::Select(select) => {
                    // Create attr path starting with parsed expr of select expression
                    let mut attr_path = expr_to_attr_path(
                        select
                            .expr()
                            .expect("Failed to get expression for select expression"),
                    )?;
                    // Walk over the children and handle them
                    for attr in select
                        .attrpath()
                        .expect("Failed to get attrpath for select expression")
                        .attrs()
                    {
                        // We have the same match arms below since we can't cooerce between Attr and Expr :(
                        attr_path.push(match attr {
                            rnix::ast::Attr::Ident(ident) => ident
                                .ident_token()
                                .expect("Failed to get ident token for ident in select expression")
                                .text()
                                .to_string(),
                            rnix::ast::Attr::Str(s) => match s.normalized_parts().as_slice() {
                                [rnix::ast::InterpolPart::Literal(s)] => s.to_string(),
                                _ => return Err(ParseFloxInstallableError::ComplexString),
                            },
                            _ => return Err(ParseFloxInstallableError::Unrecognized),
                        });
                    }
                    attr_path
                },
                // With an attrpath like `x`
                rnix::ast::Expr::Ident(ident) => vec![ident
                    .ident_token()
                    .expect("Failed to get ident token for ident expression")
                    .text()
                    .to_string()],
                // With an attrpath like `"x"`
                rnix::ast::Expr::Str(s) => vec![match s.normalized_parts().as_slice() {
                    [rnix::ast::InterpolPart::Literal(s)] => s.to_string(),
                    _ => return Err(ParseFloxInstallableError::ComplexString),
                }],
                _ => return Err(ParseFloxInstallableError::Unrecognized),
            })
        }

        // Create attr path by parsing root expression
        let attr_path = expr_to_attr_path(expr)?;

        Ok(FloxInstallable {
            source: maybe_source.map(String::from),
            attr_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flox_installable_parsing() {
        assert_eq!(
            ".#test".parse(),
            Ok(FloxInstallable {
                source: Some(".".to_string()),
                attr_path: vec!["test".to_string()]
            })
        );

        assert_eq!(
            ".#".parse(),
            Ok(FloxInstallable {
                source: Some(".".to_string()),
                attr_path: vec![]
            })
        );

        assert_eq!(
            ".".parse(),
            Ok(FloxInstallable {
                source: Some(".".to_string()),
                attr_path: vec![]
            })
        );

        assert_eq!(
            "github:NixOS/nixpkgs".parse(),
            Ok(FloxInstallable {
                source: Some("github:NixOS/nixpkgs".to_string()),
                attr_path: vec![]
            })
        );

        assert_eq!(
            "./test".parse(),
            Ok(FloxInstallable {
                source: Some("./test".to_string()),
                attr_path: vec![]
            })
        );

        assert_eq!(
            "test".parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec!["test".to_string()]
            })
        );

        assert_eq!(
            r#"x."a b""#.parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec!["x".to_string(), "a b".to_string()]
            })
        );

        assert_eq!(
            r#""a b"."a b""#.parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec!["a b".to_string(), "a b".to_string()]
            })
        );

        assert_eq!(
            r#""a b""#.parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec!["a b".to_string()]
            })
        );

        assert_eq!(
            r#""a \"b""#.parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec![r#"a "b"#.to_string()]
            })
        );

        assert_eq!(
            r#""a \\\"b""#.parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec![r#"a \"b"#.to_string()]
            })
        );

        assert_eq!(
            "a.b.c.d".parse(),
            Ok(FloxInstallable {
                source: None,
                attr_path: vec![
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                    "d".to_string()
                ]
            })
        );
    }
}
