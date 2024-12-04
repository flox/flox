use rexpect::spawn_stream;
use std::error::Error;
use std::net::TcpStream;

fn main() -> Result<(), Box<dyn Error>> {
    let tcp = TcpStream::connect("www.google.com:80")?;
    let tcp_w = tcp.try_clone()?;
    let mut session = spawn_stream(tcp, tcp_w, Some(2000));
    session.send_line("GET / HTTP/1.1")?;
    session.send_line("Host: www.google.com")?;
    session.send_line("Accept-Language: fr")?;
    session.send_line("")?;
    session.exp_string("HTTP/1.1 200 OK")?;
    Ok(())
}
