//! Attribute paths as the walker resolves them.
//!
//! The scanner resolves expressions to attribute paths (`catalogs.myorg.pkg`),
//! stopping where a component cannot be known statically. Modelling that stop
//! as a [Component] rather than a `"*"` string keeps the sentinel out of the
//! path's own vocabulary: `*` is the wire form's spelling, applied once when a
//! path is rendered.

use std::fmt::{self, Display};
use std::str::FromStr;

use rnix::ast;
use rowan::ast::AstNode;

/// One component of an [AttrPath]. Internal to the path: callers ask about
/// depth and re-root paths rather than taking them apart.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Component {
    /// A statically known attribute name.
    Attribute(String),
    /// Resolution stopped here, so anything under this point may be reached.
    Wildcard,
}

/// A name is rendered bare when Nix would accept it as one, and quoted
/// otherwise so the rendering can be read back. rnix leaves string content
/// escaped, so a name is re-quoted exactly as it was written and needs no
/// escaping of its own.
impl Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(name) if is_bare_identifier(name) => name.fmt(f),
            Self::Attribute(name) => write!(f, "\"{name}\""),
            Self::Wildcard => "*".fmt(f),
        }
    }
}

/// Whether Nix would read `name` as an identifier rather than a quoted
/// attribute: a letter or `_` followed by letters, digits, `_`, `-` or `'`.
fn is_bare_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\''))
}

/// An attribute path the walker resolved, rooted at a catalog namespace
/// parameter (`catalogs.myorg.pkg`).
///
/// A [Component::Wildcard] is always last and absorbing: nothing is known
/// about what follows a component that could not be resolved, so appending
/// past one is a no-op rather than an error.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttrPath(Vec<Component>);

impl AttrPath {
    /// A path naming a single attribute, the catalog root the walker seeds its
    /// environment with.
    pub fn root(name: impl Into<String>) -> Self {
        Self(vec![Component::Attribute(name.into())])
    }

    /// The path with `name` selected on it.
    pub fn append_attribute(mut self, name: impl Into<String>) -> Self {
        if !self.is_wildcard() {
            self.0.push(Component::Attribute(name.into()));
        }
        self
    }

    /// The path with resolution stopped at its current depth.
    pub fn append_wildcard(mut self) -> Self {
        if !self.is_wildcard() {
            self.0.push(Component::Wildcard);
        }
        self
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether resolution stopped short of naming the path's last component.
    pub fn is_wildcard(&self) -> bool {
        self.0.last() == Some(&Component::Wildcard)
    }

    /// The path with its root dropped, for reading it against a namespace
    /// other than the one it was rooted at.
    pub fn pop_root(&self) -> Self {
        Self(self.0.iter().skip(1).cloned().collect())
    }

    /// This path continued by `tail`, the inverse of [Self::pop_root] for a
    /// path being re-rooted. Appending obeys the same absorbing rule as
    /// [Self::append_attribute]: nothing follows a wildcard.
    pub fn concat(self, tail: Self) -> Self {
        tail.0
            .into_iter()
            .fold(self, |path, component| match component {
                Component::Attribute(name) => path.append_attribute(name),
                Component::Wildcard => path.append_wildcard(),
            })
    }

    /// The attribute the path is rooted at, whatever its depth.
    pub fn root_name(&self) -> Option<&str> {
        match self.0.first() {
            Some(Component::Attribute(name)) => Some(name),
            _ => None,
        }
    }
}

/// A string that is not an attribute path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not an attribute path")]
pub struct InvalidAttrPath(pub String);

/// The inverse of [Display].
///
/// A trailing wildcard is split off first: `*` lexes as multiplication, so
/// `catalogs.myorg.*` is not Nix and only the rest can be parsed as one. The
/// rest goes through rnix, so this accepts exactly what the walker records
/// from source — the same [attr_static_name] — and refuses only a dynamic
/// attribute.
impl FromStr for AttrPath {
    type Err = InvalidAttrPath;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidAttrPath(value.to_string());
        let (base, wildcard) = match value.strip_suffix(".*") {
            Some(base) => (base, true),
            None => (value, false),
        };

        let parse = rnix::Root::parse(base);
        let components = parse
            .errors()
            .is_empty()
            .then(|| parse.tree().expr())
            .flatten()
            .and_then(|expr| components_of(&expr))
            .ok_or_else(invalid)?;

        let path = Self(components);
        Ok(match wildcard {
            true => path.append_wildcard(),
            false => path,
        })
    }
}

/// The components of a select chain (`catalogs.myorg.pkg`) or a bare name.
/// Any other expression yields `None`.
fn components_of(expr: &ast::Expr) -> Option<Vec<Component>> {
    match expr {
        ast::Expr::Ident(ident) => Some(vec![Component::Attribute(ident_name(ident)?)]),
        ast::Expr::Select(select) => {
            let mut components = components_of(&select.expr()?)?;
            for attr in select.attrpath()?.attrs() {
                components.push(Component::Attribute(attr_static_name(&attr)?));
            }
            Some(components)
        },
        _ => None,
    }
}

/// The name an identifier carries.
pub(super) fn ident_name(ident: &ast::Ident) -> Option<String> {
    Some(ident.ident_token()?.text().to_string())
}

/// The name an attribute is written with, quoted or not, taken as written.
/// Whether the catalog can hold such a name is the catalog's question, not
/// this one's — [Display] quotes whatever needs it, so any name survives being
/// written out and read back.
///
/// `None` only for a dynamic attribute, which names nothing until evaluated;
/// the walker collapses those to a [Component::Wildcard].
pub(super) fn attr_static_name(attr: &ast::Attr) -> Option<String> {
    match attr {
        ast::Attr::Ident(id) => Some(id.ident_token()?.text().to_string()),
        ast::Attr::Str(s) => static_str_content(s),
        ast::Attr::Dynamic(_) => None,
    }
}

/// Extract a string node's contents when it has no interpolation, or `None`.
pub(super) fn static_str_content(s: &ast::Str) -> Option<String> {
    if s.syntax().children().next().is_some() {
        return None;
    }
    s.syntax().children_with_tokens().find_map(|n| {
        if let rowan::NodeOrToken::Token(t) = n
            && t.kind() == rnix::SyntaxKind::TOKEN_STRING_CONTENT
        {
            return Some(t.text().to_string());
        }
        None
    })
}

impl Display for AttrPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, component) in self.0.iter().enumerate() {
            if index > 0 {
                ".".fmt(f)?;
            }
            component.fmt(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_absorbs_further_components() {
        // Nothing is known past a component that could not be resolved, so a
        // selection on it names no more than the wildcard already does.
        let path = AttrPath::root("catalogs")
            .append_attribute("myorg")
            .append_wildcard()
            .append_attribute("pkg")
            .append_wildcard();
        assert_eq!(path.to_string(), "catalogs.myorg.*");
    }

    #[test]
    fn pop_root_and_concat_re_root_a_path() {
        // How a forwarded path moves between namespaces: the child's root is
        // dropped and the parent's prefix takes its place.
        let child = AttrPath::root("myorg")
            .append_attribute("toolkit")
            .append_wildcard();
        let parent = AttrPath::root("catalogs").append_attribute("myorg");
        assert_eq!(
            parent.concat(child.pop_root()).to_string(),
            "catalogs.myorg.toolkit.*"
        );
    }

    #[test]
    fn concat_onto_a_wildcard_absorbs() {
        let widened = AttrPath::root("catalogs").append_wildcard();
        let tail = AttrPath::root("myorg").append_attribute("pkg");
        assert_eq!(widened.concat(tail).to_string(), "catalogs.*");
    }
}
