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
pub(crate) struct AttrPath(Vec<Component>);

impl AttrPath {
    /// A path naming a single attribute, the catalog root the walker seeds its
    /// environment with.
    pub(crate) fn root(name: impl Into<String>) -> Self {
        Self(vec![Component::Attribute(name.into())])
    }

    /// The path with `name` selected on it.
    pub(crate) fn append_attribute(mut self, name: impl Into<String>) -> Self {
        if !self.is_wildcard() {
            self.0.push(Component::Attribute(name.into()));
        }
        self
    }

    /// The path with resolution stopped at its current depth.
    pub(crate) fn append_wildcard(mut self) -> Self {
        if !self.is_wildcard() {
            self.0.push(Component::Wildcard);
        }
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// The raw attribute names of this path, in order, with any trailing
    /// wildcard omitted. Names come back exactly as written (unquoted), for
    /// comparison against attribute names from other sources — e.g. a locked
    /// entry's `attr_path` — rather than for rendering.
    pub(crate) fn attribute_names(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|component| match component {
                Component::Attribute(name) => Some(name.as_str()),
                Component::Wildcard => None,
            })
            .collect()
    }

    /// Whether resolution stopped short of naming the path's last component.
    pub(crate) fn is_wildcard(&self) -> bool {
        self.0.last() == Some(&Component::Wildcard)
    }

    /// The path with its root dropped, for reading it against a namespace
    /// other than the one it was rooted at. `None` for a path that is only a
    /// root, which leaves nothing — every other way of building a path yields
    /// at least one component, so this is what keeps an empty one
    /// unrepresentable.
    ///
    /// A remaining lone wildcard is kept: `myorg.*` popped to `*` still says
    /// the namespace it is re-rooted onto was reached but not resolved
    /// through.
    pub(crate) fn pop_root(&self) -> Option<Self> {
        (self.len() >= 2).then(|| Self(self.0.iter().skip(1).cloned().collect()))
    }

    /// This path with `parent` in place of its root, which is how a path
    /// forwarded into an import moves back into the namespace that forwarded
    /// it. A path that is only a root becomes `parent` itself, and a trailing
    /// wildcard survives, so `myorg.*` re-rooted onto `catalogs.myorg` is
    /// `catalogs.myorg.*`.
    pub(crate) fn replace_root(&self, parent: &Self) -> Self {
        self.0
            .iter()
            .skip(1)
            .fold(parent.clone(), |path, component| match component {
                Component::Attribute(name) => path.append_attribute(name),
                Component::Wildcard => path.append_wildcard(),
            })
    }

    /// The attribute the path is rooted at, whatever its depth.
    pub(crate) fn root_name(&self) -> Option<&str> {
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
    fn replace_root_re_roots_a_forwarded_path() {
        // How a forwarded path moves between namespaces, including the two
        // ends: a trailing wildcard survives, and a path that is only a root
        // becomes the parent itself.
        let parent = AttrPath::root("catalogs").append_attribute("myorg");
        let cases = [
            (
                AttrPath::root("myorg")
                    .append_attribute("toolkit")
                    .append_wildcard(),
                "catalogs.myorg.toolkit.*",
            ),
            (
                AttrPath::root("myorg").append_wildcard(),
                "catalogs.myorg.*",
            ),
            (AttrPath::root("myorg"), "catalogs.myorg"),
        ];
        for (child, expected) in cases {
            assert_eq!(child.replace_root(&parent).to_string(), expected);
        }
    }

    #[test]
    fn pop_root_refuses_to_leave_nothing() {
        // The only way to reach an empty path, and so the only place that has
        // to refuse: every other constructor yields at least one component.
        let widened = AttrPath::root("myorg").append_wildcard();
        assert_eq!(
            widened.pop_root().map(|path| path.to_string()),
            Some("*".to_string())
        );
        assert_eq!(AttrPath::root("myorg").pop_root(), None);
    }

    #[test]
    fn appending_past_a_wildcard_is_a_no_op() {
        let widened = AttrPath::root("catalogs").append_wildcard();
        assert_eq!(widened.append_attribute("myorg").to_string(), "catalogs.*");
    }
}
