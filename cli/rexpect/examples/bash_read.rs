use rexpect::error::Error;
use rexpect::spawn_bash;

fn main() -> Result<(), Error> {
    let mut p = spawn_bash(Some(2000))?;

    // case 1: wait until program is done
    p.send_line("hostname")?;
    let hostname = p.read_line()?;
    p.wait_for_prompt()?; // go sure `hostname` is really done
    println!("Current hostname: {hostname}");

    // case 2: wait until done, only extract a few infos
    p.send_line("wc /etc/passwd")?;
    // `exp_regex` returns both string-before-match and match itself, discard first
    let (_, lines) = p.exp_regex("[0-9]+")?;
    let (_, words) = p.exp_regex("[0-9]+")?;
    let (_, bytes) = p.exp_regex("[0-9]+")?;
    p.wait_for_prompt()?; // go sure `wc` is really done
    println!("/etc/passwd has {lines} lines, {words} words, {bytes} chars");

    // case 3: read while program is still executing
    p.execute("ping 8.8.8.8", "bytes of data")?; // returns when it sees "bytes of data" in output
    for _ in 0..5 {
        // times out if one ping takes longer than 2s
        let (_, duration) = p.exp_regex("[0-9. ]+ ms")?;
        println!("Roundtrip time: {duration}");
    }
    p.send_control('c')?;
    Ok(())
}
