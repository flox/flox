# rexpect

[![Documentation](https://img.shields.io/badge/docs-master-blue.svg)][Documentation]
![License](https://img.shields.io/crates/l/rexpect.svg)
[![Crates Status](https://img.shields.io/crates/v/rexpect.svg)][Crates.io]

Spawn, control, and respond to expected patterns of child applications and
processes, enabling the automation of interactions and testing. Components
include:

- **session**: start a new process and interact with it; primary module of
  rexpect.
- **reader**: non-blocking reader, which supports waiting for strings, regex,
  and EOF.
- **process**: spawn a process in a pty.

The goal is to offer a similar set of functionality as
[pexpect](https://pexpect.readthedocs.io/en/stable/overview.html).

## Examples

[For more examples, check the examples directory.](https://github.com/philippkeller/rexpect/tree/master/examples)

### Basic usage

Simple example for interacting via ftp:

```rust,no_run
use rexpect::spawn;
use rexpect::error::*;

fn do_ftp() -> Result<(), Error> {
    let mut p = spawn("ftp speedtest.tele2.net", Some(30_000))?;
    p.exp_regex("Name \\(.*\\):")?;
    p.send_line("anonymous")?;
    p.exp_string("Password")?;
    p.send_line("test")?;
    p.exp_string("ftp>")?;
    p.send_line("cd upload")?;
    p.exp_string("successfully changed.\r\nftp>")?;
    p.send_line("pwd")?;
    p.exp_regex("[0-9]+ \"/upload\"")?;
    p.send_line("exit")?;
    p.exp_eof()?;
    Ok(())
}

fn main() {
    do_ftp().unwrap_or_else(|e| panic!("ftp job failed with {}", e));
}
```

### Example with bash and reading from programs

```rust,no_run
use rexpect::spawn_bash;
use rexpect::error::*;

fn do_bash() -> Result<(), Error> {
    let mut p = spawn_bash(Some(2000))?;

    // case 1: wait until program is done
    p.send_line("hostname")?;
    let hostname = p.read_line()?;
    p.wait_for_prompt()?; // go sure `hostname` is really done
    println!("Current hostname: {}", hostname);

    // case 2: wait until done, only extract a few infos
    p.send_line("wc /etc/passwd")?;
    // `exp_regex` returns both string-before-match and match itself, discard first
    let (_, lines) = p.exp_regex("[0-9]+")?;
    let (_, words) = p.exp_regex("[0-9]+")?;
    let (_, bytes) = p.exp_regex("[0-9]+")?;
    p.wait_for_prompt()?; // go sure `wc` is really done
    println!("/etc/passwd has {} lines, {} words, {} chars", lines, words, bytes);

    // case 3: read while program is still executing
    p.execute("ping 8.8.8.8", "bytes of data")?; // returns when it sees "bytes of data" in output
    for _ in 0..5 {
        // times out if one ping takes longer than 2s
        let (_, duration) = p.exp_regex("[0-9. ]+ ms")?;
        println!("Roundtrip time: {}", duration);
    }
    p.send_control('c')?;
    Ok(())
}

fn main() {
    do_bash().unwrap_or_else(|e| panic!("bash job failed with {}", e));
}
```

### Example with bash and job control

One frequent bitfall with sending ctrl-c and friends is that you need
to somehow ensure that the program has fully loaded, otherwise the ctrl-*
goes into nirvana. There are two functions to ensure that:

- `execute` where you need to provide a match string which is present
  on stdout/stderr when the program is ready
- `wait_for_prompt` which waits until the prompt is shown again



```rust,no_run
use rexpect::spawn_bash;
use rexpect::error::*;

fn do_bash_jobcontrol() -> Result<(), Error> {
    let mut p = spawn_bash(Some(1000))?;
    p.execute("ping 8.8.8.8", "bytes of data")?;
    p.send_control('z')?;
    p.wait_for_prompt()?;
    // bash writes 'ping 8.8.8.8' to stdout again to state which job was put into background
    p.execute("bg", "ping 8.8.8.8")?;
    p.wait_for_prompt()?;
    p.send_line("sleep 0.5")?;
    p.wait_for_prompt()?;
    // bash writes 'ping 8.8.8.8' to stdout again to state which job was put into foreground
    p.execute("fg", "ping 8.8.8.8")?;
    p.send_control('c')?;
    p.exp_string("packet loss")?;
    Ok(())
}

fn main() {
    do_bash_jobcontrol().unwrap_or_else(|e| panic!("bash with job control failed with {}", e));
}

```

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.


[Crates.io]: https://crates.io/crates/rexpect
[Documentation]: https://docs.rs/rexpect
