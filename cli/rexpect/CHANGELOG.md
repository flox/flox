# Change Log

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

<!-- next-header -->
## [Unreleased] - ReleaseDate

## [0.6.0] - 2024-11-06

### Compatibility

- `PtyProcess::get_file_handle()` returns an error if dup() fails
- `spawn_bash()` returns error if `tempfile::NamedTempFile::new()` or `write!()` fails
- Renamed `Error::SendContolError` to `Error::SendControlError`
- MSRV updated to 1.65

### Feature

- "which" functionality was added (feature gated)
- Option to skip ansi escape codes

### Fixed

- file descriptor leak was fixed in `PtyProcess::new()`
- Documentation cleanup

## [0.5.0] 2022-10-09

### Changed

- Replace error-chain with thiserror
- Make any_exp return results deterministically
- Fix argument tokenization
- Update to nix 0.25

### Fixed

- Fix https link to pexpect docs
- Many clippy fixes
- Remove unnecessary closures
- Fix EOF detection on nightly
- Fix examples

## [0.4.0] 2020-05-25

### Changed

- PtySession now works with any stream type, e.g. also tcp streams are supported now (thanks, thomasantony)
- breaking: PtyBashSession was renamed and generalized into
  PtyReplSession to allow an interface for other REPLs 
- better error messages in case of timeout to help debug when you expect
  strings which you *think* are there, but are e.g. intermixed with newlines
  or ctrl characters: newlines are printed as `\n`, carriage returns as `\r`
  and control characters as `^`
- new: `session::spawn_python`, just as a proof of concept and documentation really,
  I don't think this will be used..

### Fixed

- `spawn()` now parses single/doublequotes better. E.g. `ed -p '> '` is
  now tokenized into `["ed", "-p" "'> '"]`

## [0.3.0] 2017-10-05

### Changed

- breaking: `execute` takes string to wait for as second argument 
  (before it waited 10ms which was way too fragile)
- if process doesn't end on SIGTERM a `kill -9` is sent after timeout is elapsed

### Fixed

- ctrl-* used to consume one line. As it could be that the reader did not consume all
  output data yet this could have been a not-yet-read line. Therefore `send_control`
  no longer consumes a line.

## [0.2.0] 2017-09-20

### Changed

All `exp_*` methods now also return the yet unread string and/or the matched string:

- `exp_string`: return the yet unread string
- `exp_regex`: return a tuple of (yet unread string, matched string)
- `exp_eof` and `exp_nbytes`: return the yet unread string

### Fixed

- each execution of rexpect left a temporary file in /tmp/ this is now no longer the case
- try_read was blocking when there was no char ready (!) -> fixed

<!-- next-url -->
[Unreleased]: https://github.com/rust-cli/rexpect/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/rust-cli/argfile/compare/v0.5.0...v0.6.0
