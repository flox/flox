use rnix::ast::HasEntry;
use rnix::{NodeOrToken, SyntaxNode};
use rowan::ast::AstNode;
use rowan::{GreenNode, GreenNodeBuilder, GreenToken, SyntaxKind};

pub trait AttrSetExt {
    fn new() -> Self;
    fn insert_unchecked<A: AsRef<str>>(
        &self,
        path: impl IntoIterator<Item = A>,
        value: SyntaxNode,
    ) -> Self;

    fn update_or_insert<A: AsRef<str> + Clone>(
        &self,
        path: impl IntoIterator<Item = A>,
        value: SyntaxNode,
    ) -> Self;

    fn find_by_path<A: AsRef<str>>(
        &self,
        path: impl IntoIterator<Item = A>,
    ) -> Option<rnix::ast::Expr>;
}

impl AttrSetExt for rnix::ast::AttrSet {
    fn new() -> Self {
        let green = GreenNode::new(SyntaxKind(rnix::ast::AttrSet::KIND as u16), [
            NodeOrToken::Token(GreenToken::new(
                SyntaxKind(rnix::SyntaxKind::TOKEN_L_BRACE as u16),
                "{",
            )),
            NodeOrToken::Token(GreenToken::new(
                SyntaxKind(rnix::SyntaxKind::TOKEN_R_BRACE as u16),
                "}",
            )),
        ]);
        rnix::ast::AttrSet::cast(SyntaxNode::new_root(green)).unwrap()
    }

    fn insert_unchecked<A: AsRef<str>>(
        &self,
        path: impl IntoIterator<Item = A>,
        value: SyntaxNode,
    ) -> Self {
        let value = GreenNode::new(SyntaxKind(rnix::ast::AttrpathValue::KIND as u16), [
            NodeOrToken::Node(rnix::ast::Attrpath::new(path).syntax().green().into_owned()),
            NodeOrToken::Token(GreenToken::new(
                SyntaxKind(rnix::SyntaxKind::TOKEN_ASSIGN as u16),
                "=",
            )),
            NodeOrToken::Node(value.green().into_owned()),
            NodeOrToken::Token(GreenToken::new(
                SyntaxKind(rnix::SyntaxKind::TOKEN_SEMICOLON as u16),
                ";",
            )),
        ]);

        rnix::ast::AttrSet::cast(SyntaxNode::new_root(
            self.syntax()
                .green()
                .insert_child(1, NodeOrToken::Node(value)),
        ))
        .unwrap()
    }

    fn update_or_insert<A: AsRef<str> + Clone>(
        &self,
        path: impl IntoIterator<Item = A>,
        value: SyntaxNode,
    ) -> Self {
        let path: Vec<_> = path.into_iter().collect();

        match self.clone_subtree().find_by_path(path.clone()) {
            None => self.insert_unchecked(path, value),
            Some(before) => {
                let index = before.syntax().index();
                let replace = before
                    .syntax()
                    .parent()
                    .unwrap()
                    .green()
                    .replace_child(index, NodeOrToken::Node(value.green().into_owned()));

                rnix::ast::AttrSet::cast(SyntaxNode::new_root(
                    before.syntax().parent().unwrap().replace_with(replace),
                ))
                .unwrap()
            },
        }
    }

    fn find_by_path<A: AsRef<str>>(
        &self,
        path: impl IntoIterator<Item = A>,
    ) -> Option<rnix::ast::Expr> {
        let search: Vec<_> = path
            .into_iter()
            .map(|a| rnix::ast::Attr::new(a.as_ref()))
            .map(|a| a.to_string())
            .collect();

        'attrpaths: for attrpath_value in self.attrpath_values() {
            let value = attrpath_value.value().unwrap();
            let mut attrpath = attrpath_value
                .attrpath()
                .unwrap()
                .attrs()
                .peekable()
                .map(|attr| attr.to_string());

            let mut search_iter = search.iter();

            while let Some(search_attr) = search_iter.next() {
                match attrpath.next() {
                        Some(ref attr)  if attr == search_attr => continue,
                        Some(_)  /* else */             => continue 'attrpaths,
                        None => {

                            if let rnix::ast::Expr::AttrSet(attrset) = value {
                                let in_subattrset = attrset.find_by_path([search_attr].into_iter().chain(search_iter.clone()));
                                if in_subattrset.is_some() {
                                    return in_subattrset;
                                }
                                else {
                                    continue 'attrpaths;
                                }
                            }
                        },
                    }
            }
            // exact match
            return Some(value);
        }

        None
    }
}

pub trait StrExt {
    fn new(value: &str) -> Self;
}
impl StrExt for rnix::ast::Str {
    fn new(value: &str) -> Self {
        let mut node = GreenNodeBuilder::new();
        node.start_node(rowan::SyntaxKind(rnix::ast::Str::KIND as u16));
        node.token(
            SyntaxKind(rnix::SyntaxKind::TOKEN_STRING_START as u16),
            "\"",
        );
        node.token(
            SyntaxKind(rnix::SyntaxKind::TOKEN_STRING_CONTENT as u16),
            value,
        );
        node.token(SyntaxKind(rnix::SyntaxKind::TOKEN_STRING_END as u16), "\"");

        node.finish_node();

        let green = node.finish();

        rnix::ast::Str::cast(SyntaxNode::new_root(green)).unwrap()
    }
}

pub trait IdentExt {
    fn new(value: &str) -> Self;
}
impl IdentExt for rnix::ast::Ident {
    fn new(value: &str) -> Self {
        let mut node = GreenNodeBuilder::new();

        node.start_node(SyntaxKind(rnix::ast::Ident::KIND as u16));
        node.token(SyntaxKind(rnix::SyntaxKind::TOKEN_IDENT as u16), value);
        node.finish_node();
        let green = node.finish();
        rnix::ast::Ident::cast(SyntaxNode::new_root(green)).unwrap()
    }
}

pub trait AttrExt {
    fn new(value: &str) -> Self;
}
impl AttrExt for rnix::ast::Attr {
    fn new(value: &str) -> Self {
        if value.chars().all(|c| c.is_ascii_alphabetic()) {
            rnix::ast::Attr::Ident(rnix::ast::Ident::new(value))
        } else {
            rnix::ast::Attr::Str(rnix::ast::Str::new(value))
        }
    }
}

pub trait AttrpathExt {
    fn new<A: AsRef<str>>(package: impl IntoIterator<Item = A>) -> Self;
}

impl AttrpathExt for rnix::ast::Attrpath {
    fn new<A: AsRef<str>>(package: impl IntoIterator<Item = A>) -> Self {
        let nodes = package
            .into_iter()
            .map(|a| rnix::ast::Attr::new(a.as_ref()))
            .map(|attr| attr.syntax().green().into_owned())
            .map(NodeOrToken::Node);

        let with_dots = itertools::intersperse(
            nodes,
            NodeOrToken::Token(GreenToken::new(
                SyntaxKind(rnix::SyntaxKind::TOKEN_DOT as u16),
                ".",
            )),
        )
        .collect::<Vec<_>>();

        let green = GreenNode::new(SyntaxKind(rnix::ast::Attrpath::KIND as u16), with_dots);

        rnix::ast::Attrpath::cast(SyntaxNode::new_root(green)).unwrap()
    }
}

pub fn as_green_children(
    node: &SyntaxNode,
) -> impl Iterator<Item = NodeOrToken<GreenNode, GreenToken>> {
    node.children_with_tokens().map(|element| match element {
        NodeOrToken::Node(n) => NodeOrToken::Node(n.green().into_owned()),
        NodeOrToken::Token(t) => NodeOrToken::Token(GreenToken::new(t.green().kind(), t.text())),
    })
}

#[cfg(test)]
mod tests {
    use rnix::ast::{AttrSet, Expr, Str};

    use super::*;
    #[test]
    fn test_insert() {
        let set = AttrSet::cast(
            rnix::Root::parse("{}")
                .tree()
                .expr()
                .unwrap()
                .syntax()
                .clone(),
        )
        .unwrap();

        // abc.xyz = { version = \"aewq\"; };

        let new = set.insert_unchecked(["hello"], Str::new("world").syntax().clone());
        assert_eq!(new.to_string(), "{hello=\"world\";}");

        let new = set.insert_unchecked(["hello", "there"], Str::new("world").syntax().clone());
        assert_eq!(new.to_string(), "{hello.there=\"world\";}");
    }

    #[test]
    fn test_updating_insert() {
        let set = AttrSet::cast(
            rnix::Root::parse("{abc.xyz={hello=\"world\";};}")
                .tree()
                .expr()
                .unwrap()
                .syntax()
                .clone(),
        )
        .unwrap();

        // abc.xyz = { version = \"aewq\"; };

        let new = set.update_or_insert(["abc", "xyz", "hello"], Str::new("ciao").syntax().clone());
        assert_eq!(new.to_string(), "{abc.xyz={hello=\"ciao\";};}");

        let new = set.update_or_insert(["abc", "xyz", "hallo"], Str::new("welt").syntax().clone());
        assert_eq!(
            new.to_string(),
            "{abc.xyz.hallo=\"welt\";abc.xyz={hello=\"world\";};}"
        );
    }

    #[test]

    fn find() {
        let set = AttrSet::cast(
            rnix::Root::parse(
                "
            {
                hello = {
                    there = \"world\";
                };
            }
            ",
            )
            .tree()
            .expr()
            .unwrap()
            .syntax()
            .clone(),
        )
        .unwrap();

        assert_eq!(
            set.find_by_path(["hello", "there"]).unwrap().to_string(),
            "\"world\"".to_string()
        );

        assert_eq!(set.find_by_path(["hello", "where"]), None);
    }

    #[test]
    fn test_utils() {
        let expr = rnix::Root::parse("{abc.xyz = { version = \"xyz\"; };}")
            .ok()
            .unwrap();

        let version = dbg!(
            rnix::ast::AttrSet::cast(expr.expr().unwrap().syntax().clone())
                .unwrap()
                .find_by_path(["abc", "xyz", "version"])
        )
        .unwrap();

        assert_eq!(version.to_string(), Expr::Str(Str::new("xyz")).to_string());

        // let version_index = version.syntax().index();

        let expr = &rnix::ast::AttrSet::cast(expr.expr().unwrap().syntax().clone())
            .unwrap()
            .update_or_insert(
                ["abc", "xyz", "version"],
                AttrSet::new().syntax().to_owned(),
            );

        let expr = expr.update_or_insert(
            ["abc", "xyz", "not-version"],
            AttrSet::new().syntax().to_owned(),
        );

        let expr = expr.insert_unchecked(
            ["abc", "xyz", "not-version", "inner"],
            AttrSet::new().syntax().to_owned(),
        );

        assert_eq!(
            expr.to_string(),
            r#"{abc.xyz."not-version".inner={};abc.xyz."not-version"={};abc.xyz = { version = {}; };}"#
        )
    }
}
