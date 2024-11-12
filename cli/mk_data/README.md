# mk_data

This program generates test data used for mocking during unit tests.
Cases are specified in a config file.

```
Generate mock test data from a config file

Usage: mk_data [OPTIONS] <PATH>

Arguments:
  <PATH>  The path to the config file

Options:
  -f, --force            Regenerate all data and overwrite existing data
  -o, --output <OUTPUT>  The path to the directory in which to store the output [default: $PWD/generated]
  -i, --input <INPUT>    The path to the directory containing the input data [default: $PWD/input_data]
  -q, --quiet            Don't show a spinner
  -h, --help             Print help
```

## Config file

The `[vars]` section is special in that it doesn't specify jobs to run or
generate output.

The directory structure generated is as follows:
```
<root>/
    init/
        <job_name>.json
    resolve/
        <job_name>.json
    search/
        <job_name>.json
    show/
        <job_name>.json
    envs/
        <job_name>/
            <job_name.json>
            manifest.toml
            manifest.lock
```

The only real exception here is that the directories in the `envs` section
are created manually using the job config.

### Job config

Each job is executed in a temporary directory inside the output directory.

The config for a job is as follows:
```
files                  := [str] | null
pre_cmd                := str | null
cmd                    := str | null
post_cmd               := str | null
ignore_pre_cmd_errors  := bool | null
ignore_cmd_errors      := bool | null
ignore_post_cmd_errors := bool | null
skip_if_output_exists  := str | null
```
where `*cmd`s are run by Bash.

Semantics:
- If `skip_if_output_exists` is `<path>`, then `$output_dir/<path>` is checked and if it exists the job is skipped.
    - This is necessary if the job puts output in a place other than `<category>/<name>.json`.
- `files` are relative paths specified relative to the input data directory.
- `files` are copied from the input data directory into the working directory before anything else happens.
- `pre_cmd` executes next, but no response file is generated during execution.
- `cmd` is executed next, and a response file is generated.
- `post_cmd` is executed next, but no response file is generated.
- `$RESPONSE_FILE` contains the path to the generated response file and is available inside `post_cmd` for post-processing of response files.

Any error encountered while copying files or running commands will fail the job
unless the appropriate `ignore_*_errors` field is set.
If you expect a command to fail and still want to record the response, you should
run the command as `cmd || true`.

### `[vars]`
Contains environment variables that are set for all jobs.
This is useful if you want to set a particular catalog server URL e.g. preview vs. production.

### `[resolve]`
Intended to hold jobs that hit the `/resolve` endpoint, which is mostly installs.

### `[search]` and `[show]`
Intended to hold jobs using `flox search` and `flox show`.

### `[init]`
Intended to hold jobs that call `flox init`.
This section makes heavy use of `files` and `pre_cmd` to put language-specific
files in the right place before calling `flox init --auto-setup`.

### `[envs]`
Intended to hold jobs that lock manifests.
The idea here is to start providing manifest/lockfile pairs that can be used
as fixtures in other tests without needing to build environments all the time.
These tests manually move files around into directories named after the job
and record the manifest, lockfile, and response recorded during resolution.

## Is it pretty?
Yes, it has a spinner, but if you're a scrooge or a log file you can turn it off
with the `-q` flag.
