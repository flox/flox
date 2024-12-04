use rexpect::error::Error;
use rexpect::spawn_bash;

fn main() -> Result<(), Error> {
    let mut p = spawn_bash(Some(1000))?;
    p.execute("ping 8.8.8.8", "bytes")?;
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
