//! The starter README written into new environments.
//!
//! `flox init` scaffolds this template into `.flox/env/README.md`, and
//! `flox edit --readme` seeds it when an environment has no README yet. The
//! README documents what an environment provides and is rendered by
//! `flox info`, on `flox activate`, and on FloxHub.
use indoc::formatdoc;

/// A starter README for an environment named `name`.
pub fn template(name: &str) -> String {
    formatdoc! {r#"
        # {name}

        Describe what this environment provides and how to use it. This file is
        shown by `flox info`, on `flox activate`, and on FloxHub, so it is a good
        place to tell teammates (and AI agents) how to get started.

        ## Getting started

        Document the commands a user should run once the environment is active.
        For example:

        ```
        # start the development server
        ```

        ## What's included

        List the key tools and services this environment sets up.
    "#}
}
