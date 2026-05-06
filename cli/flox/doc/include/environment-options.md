## Environment Options

If no environment is specified for an environment command,
the environment in the current directory
or the active environment that was last activated is used.

`-d`, `--dir`
:   Path containing a .flox/ directory.

`-r`, `--reference`
:   A FloxHub environment, specified in the form `<owner>/<name>`.

`-D`, `--default`
:   Use your default environment (`<your-user>/default`).
    When unauthenticated in an interactive context, you will be prompted to
    log in.
    In non-interactive contexts (e.g., scripts or CI), this flag will fail
    with an error when authentication is missing.
