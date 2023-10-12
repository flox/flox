## Release 0.3.5 (2023-10-12)

This release addresses several bugs and improvements, including:

- Added support for sh in `flox activate`.
- Updated error message for `flox install`.
- Worked on changes in the background to the flox CLI as we aim for our general availability (1.0) release.

As we prepare new updates to the CLI, the -e flag and other legacy behavior related to environments is now only 
available by when `--bash-passthru` option is specified or the `FLOX_BASH_PASSTHRU` environment variable is set 
to `true`. The team has begun implementing the -e flag's replacement: `--dir` and `--remote` flags that make 
behavior more clear and helps us align to a single environment no matter if it's managed by a service or managed 
in files. We apologize for the disruption in the beta and hope you will continue to upgrade and give us feedback 
as we transition!

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
