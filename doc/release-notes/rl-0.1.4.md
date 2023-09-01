## Release 0.1.4 (2023-04-20)

This release includes several performance improvements, bug fixes, and feature refinements based on feedback from our users, including:

- Ported multiple functions from bash MVP to rust rewrite including channel management (subscribe/unsubscribe).
- Added support for bare clones of floxmeta repositories to allow for future enhancements.
    - Please note potential for backwards compatibility issues with the earliest versions of flox.
      If you see the error `fatal: path 'floxUserMeta.json' does not exist in 'floxmain'` then please run the following:
      ```
      mv ~/.cache/flox/meta/local /tmp
      ln -s your_github_handle ~/.cache/flox/meta/local
      ```
- Updated output level of internal commands to forward more prompts and warnings to users, e.g. to provide passwords or yubikey input.
- Squashed some bugs, improved error messages / UX output, updated manpages and fixed typos.

We especially want to thank all our <a href="https://github.com/flox/flox">github.com/flox/flox</a> contributors and <a href="https://discourse.flox.dev">discourse community</a> members for all your valuable feedback!
