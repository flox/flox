## Release 0.1.5 (2023-05-04)

This release includes several bug fixes and feature refinements based on feedback from our users, including:

- Updated `flox edit` to persist changes made to comments within declarative manifests.
- Updated `flox create` to create an initial generation for the empty environment. This means `flox create` && `flox install` 
  will result in two generations, whereas a plain `flox install` will only create a single generation.
- The `FLOX_ENV` environment variable is now set and points to the path of the most recently activated flox environment. 
  This can be used in environment hooks to set other variables to paths within the environment, such as `PYTHONPATH`.
- Squashed some bugs, supressed superfluous warnings, and improved error messages / UX output.

We especially want to thank all our <a href="https://github.com/flox/flox">github.com/flox/flox</a> contributors and 
<a href="https://discourse.floxdev.com">discourse community</a> members for all your valuable feedback!
