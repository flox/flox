use rexpect::error::Error;
use rexpect::process::wait;
use rexpect::spawn;

/// The following code emits:
/// cat exited with code 0, all good!
/// cat exited with code 1
/// Output (stdout and stderr): cat: /this/does/not/exist: No such file or directory
fn main() -> Result<(), Error> {
    let p = spawn("cat /etc/passwd", Some(2000))?;
    match p.process.wait() {
        Ok(wait::WaitStatus::Exited(_, 0)) => println!("cat exited with code 0, all good!"),
        _ => println!("cat exited with code >0, or it was killed"),
    }

    let mut p = spawn("cat /this/does/not/exist", Some(2000))?;
    match p.process.wait() {
        Ok(wait::WaitStatus::Exited(_, 0)) => println!("cat succeeded"),
        Ok(wait::WaitStatus::Exited(_, c)) => {
            println!("Cat failed with exit code {c}");
            println!("Output (stdout and stderr): {}", p.exp_eof()?);
        }
        // for other possible return types of wait()
        // see here: https://tailhook.github.io/rotor/nix/sys/wait/enum.WaitStatus.html
        _ => println!("cat was probably killed"),
    }

    Ok(())
}
