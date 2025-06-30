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

## Output

The directory structure generated is as follows:
```
<root>/
    resolve/
        <job_name>.yaml
    search/
        <job_name>.yaml
    show/
        <job_name>.yaml
    lock/
        <job_name>.lock
    env/
        <job_name>/
            .flox/*
            resp.yaml
    init/
        <job_name>/
            .flox/*
            resp.yaml
    custom/
        <job_name>/
            .flox/*
            resp.yaml
```

This structure means that in order to mark a job as needing to be re-run, you just need to delete one thing: either the `<job name>.(yaml | lock)` file, or the `<job name>` directory depending on the job type.

## Job config

Each job is executed in a temporary directory inside the output directory.
Each job category also has a category-specific configuration syntax as describe below.

Most of the jobs have the same flow and are noted where they differ:
- Run `flox init` (implicit)
- Run `pre_cmd`
- Run the category-specific command (implicit)
- Run `post_cmd`
- If `ignore_errors` is specified, errors in the category-specific command, `pre_cmd`, and `post_cmd` are ignored.

### Resolve

```
pkgs          := [ str ]
pre_cmd       := str | null
post_cmd      := str | null
ignore_errors := bool | null
```

Runs a `flox install` command with `pkgs` as the argument, so you can also list `-i` as a "package" and it will also work.

### Search

```
query         := str
all           := bool | null
pre_cmd       := str | null
post_cmd      := str | null
ignore_errors := bool | null
```

Runs a `flox search` with `query` as the argument.

### Show

```
query         := str
pre_cmd       := str | null
post_cmd      := str | null
ignore_errors := bool | null
```

Runs a `flox show` with `query` as the argument.

### Lock

```
manifest := str
```

Runs a `flox lock-manifest` with `manifest`, where the string provided to `manifest` is the name of a manifest in `INPUT_DATA/manifests`.

### Env

```
manifest := str
```

Runs a `flox edit -f` with `manifest`, where the string provided to `manifest` is the name of a manifest in `INPUT_DATA/manifests`.

### Init

```
unpack_dir_contents := [ str ] | null
auto_setup          := bool | null
pre_cmd             := str | null
post_cmd            := str | null
ignore_errors       := bool | null
```

Since the point of this job is to run `flox init`, there is no implicit `flox init` step run before `pre_cmd`.
Instead, the flow looks like this:
- Unpack the contents of each directory in `unpack_dir_contents` into the current directory.
- Run `pre_cmd`.
- Run `flox init` (implicit)
- Run `post_cmd`

The strings provided to `unpack_dir_contents` are specified relative to `INPUT_DATA`.

### Custom

```
unpack_dir_contents := [ str ] | null
pre_cmd             := str | null
record_cmd          := str | null
post_cmd            := str | null
ignore_errors       := bool | null
```

This job is for things that don't fit nicely into the other categories, so it allows more flexibility.
There is also no implicit `flox init` step, so if you need an environment, you must add that via `pre_cmd`.
`record_cmd` is executed between `pre_cmd` and `post_cmd` and is the command whose responses are recorded if desired.

## Environment variables

- Session-wide environment variables may be set with the `[vars]` section.
- `$RESPONSE_FILE` contains the path to the generated response file and is available inside `post_cmd` for post-processing of response files.
- `$INPUT_DATA` is available inside all commands of `[custom]` jobs.

## Is it pretty?
Yes, it has a spinner, but if you're a scrooge or a log file you can turn it off
with the `-q` flag.
