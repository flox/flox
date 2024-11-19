//! Unblocking reader which supports waiting for strings/regexes and EOF to be present

use crate::error::Error;
pub use regex::Regex;
use std::io::prelude::*;
use std::io::{self, BufReader};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::{fmt, time};

#[derive(Debug)]
enum PipeError {
    IO(io::Error),
}

#[derive(Debug)]
#[allow(clippy::upper_case_acronyms)]
enum PipedChar {
    Char(u8),
    EOF,
}

pub enum ReadUntil {
    String(String),
    Regex(Regex),
    EOF,
    NBytes(usize),
    Any(Vec<ReadUntil>),
}

impl fmt::Display for ReadUntil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let printable = match self {
            ReadUntil::String(ref s) if s == "\n" => "\\n (newline)".to_owned(),
            ReadUntil::String(ref s) if s == "\r" => "\\r (carriage return)".to_owned(),
            ReadUntil::String(ref s) => format!("\"{s}\""),
            ReadUntil::Regex(ref r) => format!("Regex: \"{r}\""),
            ReadUntil::EOF => "EOF (End of File)".to_owned(),
            ReadUntil::NBytes(n) => format!("reading {n} bytes"),
            ReadUntil::Any(ref v) => {
                let mut res = Vec::new();
                for r in v {
                    res.push(r.to_string());
                }
                res.join(", ")
            }
        };
        write!(f, "{printable}")
    }
}

/// find first occurrence of needle within buffer
///
/// # Arguments:
///
/// - buffer: the currently read buffer from a process which will still grow in the future
/// - eof: if the process already sent an EOF or a HUP
///
/// # Return
///
/// Tuple with match positions:
/// 1. position before match (0 in case of EOF and Nbytes)
/// 2. position after match
pub fn find(needle: &ReadUntil, buffer: &str, eof: bool) -> Option<(usize, usize)> {
    match needle {
        ReadUntil::String(ref s) => buffer.find(s).map(|pos| (pos, pos + s.len())),
        ReadUntil::Regex(ref pattern) => pattern.find(buffer).map(|mat| (mat.start(), mat.end())),
        ReadUntil::EOF => {
            if eof {
                Some((0, buffer.len()))
            } else {
                None
            }
        }
        ReadUntil::NBytes(n) => {
            if *n <= buffer.len() {
                Some((0, *n))
            } else if eof && !buffer.is_empty() {
                // reached almost end of buffer, return string, even though it will be
                // smaller than the wished n bytes
                Some((0, buffer.len()))
            } else {
                None
            }
        }
        ReadUntil::Any(ref anys) => anys
            .iter()
            // Filter matching needles
            .filter_map(|any| find(any, buffer, eof))
            // Return the left-most match
            .min_by(|(start1, end1), (start2, end2)| {
                if start1 == start2 {
                    end1.cmp(end2)
                } else {
                    start1.cmp(start2)
                }
            }),
    }
}

/// Options for `NBReader`
///
/// - timeout:
///  + `None`: `read_until` is blocking forever. This is probably not what you want
///  + `Some(millis)`: after millis milliseconds a timeout error is raised
/// - `strip_ansi_escape_codes`: Whether to filter out escape codes, such as colors.
#[derive(Default)]
pub struct Options {
    pub timeout_ms: Option<u64>,
    pub strip_ansi_escape_codes: bool,
}

/// Non blocking reader
///
/// Typically you'd need that to check for output of a process without blocking your thread.
/// Internally a thread is spawned and the output is read ahead so when
/// calling `read_line` or `read_until` it reads from an internal buffer
pub struct NBReader {
    reader: Receiver<Result<PipedChar, PipeError>>,
    pub buffer: String,
    eof: bool,
    timeout: Option<time::Duration>,
}

impl NBReader {
    /// Create a new reader instance
    ///
    /// # Arguments:
    ///
    /// - f: file like object
    /// - options: see `Options`
    pub fn new<R: Read + Send + 'static>(f: R, options: Options) -> NBReader {
        let (tx, rx) = channel();

        // spawn a thread which reads one char and sends it to tx
        thread::spawn(move || -> Result<(), Error> {
            let mut reader = BufReader::new(f);
            let mut byte = [0u8];
            let mut in_escape_code = false;

            loop {
                match reader.read(&mut byte) {
                    Ok(0) => {
                        tx.send(Ok(PipedChar::EOF))
                            .map_err(|_| Error::MpscSendError)?;
                        break;
                    }
                    Ok(_) => {
                        if options.strip_ansi_escape_codes && byte[0] == 27 {
                            in_escape_code = true;
                        } else if options.strip_ansi_escape_codes && in_escape_code {
                            if char::from(byte[0]).is_alphabetic() {
                                in_escape_code = false;
                            }
                        } else {
                            tx.send(Ok(PipedChar::Char(byte[0])))
                                .map_err(|_| Error::MpscSendError)?;
                        }
                    }
                    Err(error) => {
                        tx.send(Err(PipeError::IO(error)))
                            .map_err(|_| Error::MpscSendError)?;
                    }
                }
            }
            Ok(())
            // don't do error handling as on an error it was most probably
            // the main thread which exited (remote hangup)
        });
        // allocate string with a initial capacity of 1024, so when appending chars
        // we don't need to reallocate memory often
        NBReader {
            reader: rx,
            buffer: String::with_capacity(1024),
            eof: false,
            timeout: options.timeout_ms.map(time::Duration::from_millis),
        }
    }

    /// reads all available chars from the read channel and stores them in self.buffer
    fn read_into_buffer(&mut self) -> Result<(), Error> {
        if self.eof {
            return Ok(());
        }
        while let Ok(from_channel) = self.reader.try_recv() {
            match from_channel {
                Ok(PipedChar::Char(c)) => self.buffer.push(c as char),
                Ok(PipedChar::EOF) => self.eof = true,
                // this is just from experience, e.g. "sleep 5" returns the other error which
                // most probably means that there is no stdout stream at all -> send EOF
                // this only happens on Linux, not on OSX
                Err(PipeError::IO(ref err)) => {
                    // For an explanation of why we use `raw_os_error` see:
                    // https://github.com/zhiburt/ptyprocess/commit/df003c8e3ff326f7d17bc723bc7c27c50495bb62
                    self.eof = err.raw_os_error() == Some(5);
                }
            }
        }
        Ok(())
    }

    /// Read until needle is found (blocking!) and return tuple with:
    /// 1. yet unread string until and without needle
    /// 2. matched needle
    ///
    /// This methods loops (while reading from the Cursor) until the needle is found.
    ///
    /// There are different modes:
    ///
    /// - `ReadUntil::String` searches for string (use '\n'.`to_string()` to search for newline).
    ///   Returns not yet read data in first String, and needle in second String
    /// - `ReadUntil::Regex` searches for regex
    ///   Returns not yet read data in first String and matched regex in second String
    /// - `ReadUntil::NBytes` reads maximum n bytes
    ///   Returns n bytes in second String, first String is left empty
    /// - `ReadUntil::EOF` reads until end of file is reached
    ///   Returns all bytes in second String, first is left empty
    ///
    /// Note that when used with a tty the lines end with \r\n
    ///
    /// Returns error if EOF is reached before the needle could be found.
    ///
    /// # Example with line reading, byte reading, regex and EOF reading.
    ///
    /// ```
    /// # use std::io::Cursor;
    /// use rexpect::reader::{NBReader, ReadUntil, Regex, Options};
    /// // instead of a Cursor you would put your process output or file here
    /// let f = Cursor::new("Hello, miss!\n\
    ///                         What do you mean: 'miss'?");
    /// let mut e = NBReader::new(f, Options::default());
    ///
    /// let (first_line, _) = e.read_until(&ReadUntil::String('\n'.to_string())).unwrap();
    /// assert_eq!("Hello, miss!", &first_line);
    ///
    /// let (_, two_bytes) = e.read_until(&ReadUntil::NBytes(2)).unwrap();
    /// assert_eq!("Wh", &two_bytes);
    ///
    /// let re = Regex::new(r"'[a-z]+'").unwrap(); // will find 'miss'
    /// let (before, reg_match) = e.read_until(&ReadUntil::Regex(re)).unwrap();
    /// assert_eq!("at do you mean: ", &before);
    /// assert_eq!("'miss'", &reg_match);
    ///
    /// let (_, until_end) = e.read_until(&ReadUntil::EOF).unwrap();
    /// assert_eq!("?", &until_end);
    /// ```
    ///
    pub fn read_until(&mut self, needle: &ReadUntil) -> Result<(String, String), Error> {
        let start = time::Instant::now();

        loop {
            self.read_into_buffer()?;

            if let Some(tuple_pos) = find(needle, &self.buffer, self.eof) {
                let first = self.buffer.drain(..tuple_pos.0).collect();
                let second = self.buffer.drain(..tuple_pos.1 - tuple_pos.0).collect();
                return Ok((first, second));
            }

            // reached end of stream and didn't match -> error
            // we don't know the reason of eof yet, so we provide an empty string
            // this will be filled out in session::exp()
            if self.eof {
                return Err(Error::EOF {
                    expected: needle.to_string(),
                    got: self.buffer.clone(),
                    exit_code: None,
                });
            }

            // ran into timeout
            if let Some(timeout) = self.timeout {
                if start.elapsed() > timeout {
                    return Err(Error::Timeout {
                        expected: needle.to_string(),
                        got: self
                            .buffer
                            .clone()
                            .replace('\n', "`\\n`\n")
                            .replace('\r', "`\\r`")
                            .replace('\u{1b}', "`^`"),
                        timeout,
                    });
                }
            }
            // nothing matched: wait a little
            thread::sleep(time::Duration::from_millis(5));
        }
    }

    /// Try to read one char from internal buffer. Returns None if
    /// no char is ready, Some(char) otherwise. This is non-blocking
    pub fn try_read(&mut self) -> Option<char> {
        // discard eventual errors, EOF will be handled in read_until correctly
        let _ = self.read_into_buffer();
        if !self.buffer.is_empty() {
            self.buffer.drain(..1).last()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expect_melon() {
        let f = io::Cursor::new("a melon\r\n");
        let mut r = NBReader::new(f, Options::default());
        assert_eq!(
            ("a melon".to_owned(), "\r\n".to_owned()),
            r.read_until(&ReadUntil::String("\r\n".to_owned()))
                .expect("cannot read line")
        );
        // check for EOF
        match r.read_until(&ReadUntil::NBytes(10)) {
            Ok(_) => panic!(),
            Err(Error::EOF { .. }) => {}
            Err(_) => panic!(),
        }
    }

    #[test]
    fn test_regex() {
        let f = io::Cursor::new("2014-03-15");
        let mut r = NBReader::new(f, Options::default());
        let re = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
        assert_eq!(
            ("".to_owned(), "2014-03-15".to_owned()),
            r.read_until(&ReadUntil::Regex(re))
                .expect("regex doesn't match")
        );
    }

    #[test]
    fn test_regex2() {
        let f = io::Cursor::new("2014-03-15");
        let mut r = NBReader::new(f, Options::default());
        let re = Regex::new(r"-\d{2}-").unwrap();
        assert_eq!(
            ("2014".to_owned(), "-03-".to_owned()),
            r.read_until(&ReadUntil::Regex(re))
                .expect("regex doesn't match")
        );
    }

    #[test]
    fn test_nbytes() {
        let f = io::Cursor::new("abcdef");
        let mut r = NBReader::new(f, Options::default());
        assert_eq!(
            ("".to_owned(), "ab".to_owned()),
            r.read_until(&ReadUntil::NBytes(2)).expect("2 bytes")
        );
        assert_eq!(
            ("".to_owned(), "cde".to_owned()),
            r.read_until(&ReadUntil::NBytes(3)).expect("3 bytes")
        );
        assert_eq!(
            ("".to_owned(), "f".to_owned()),
            r.read_until(&ReadUntil::NBytes(4)).expect("4 bytes")
        );
    }

    #[test]
    fn test_any_with_multiple_possible_matches() {
        let f = io::Cursor::new("zero one two three four five");
        let mut r = NBReader::new(f, Options::default());

        let result = r
            .read_until(&ReadUntil::Any(vec![
                ReadUntil::String("two".to_owned()),
                ReadUntil::String("one".to_owned()),
            ]))
            .expect("finding string");

        assert_eq!(("zero ".to_owned(), "one".to_owned()), result);
    }

    #[test]
    fn test_any_with_same_start_different_length() {
        let f = io::Cursor::new("hi hello");
        let mut r = NBReader::new(f, Options::default());

        let result = r
            .read_until(&ReadUntil::Any(vec![
                ReadUntil::String("hello".to_owned()),
                ReadUntil::String("hell".to_owned()),
            ]))
            .expect("finding string");

        assert_eq!(("hi ".to_owned(), "hell".to_owned()), result);
    }

    #[test]
    fn test_eof() {
        let f = io::Cursor::new("lorem ipsum dolor sit amet");
        let mut r = NBReader::new(f, Options::default());
        r.read_until(&ReadUntil::NBytes(2)).expect("2 bytes");
        assert_eq!(
            ("".to_owned(), "rem ipsum dolor sit amet".to_owned()),
            r.read_until(&ReadUntil::EOF).expect("reading until EOF")
        );
    }

    #[test]
    fn test_skip_partial_ansi_code() {
        let f = io::Cursor::new("\x1b[31;1;4mHello\x1b[1");
        let mut r = NBReader::new(
            f,
            Options {
                timeout_ms: None,
                strip_ansi_escape_codes: true,
            },
        );
        let bytes = r
            .read_until(&ReadUntil::String("Hello".to_owned()))
            .unwrap();
        assert_eq!(bytes, ("".to_owned(), "Hello".to_owned()));
        assert_eq!(None, r.try_read());
    }

    #[test]
    fn test_skip_ansi_codes() {
        let f = io::Cursor::new("\x1b[31;1;4mHello\x1b[0m");
        let mut r = NBReader::new(
            f,
            Options {
                timeout_ms: None,
                strip_ansi_escape_codes: true,
            },
        );
        let bytes = r
            .read_until(&ReadUntil::String("Hello".to_owned()))
            .unwrap();
        assert_eq!(bytes, ("".to_owned(), "Hello".to_owned()));
        assert_eq!(None, r.try_read());
    }

    #[test]
    fn test_try_read() {
        let f = io::Cursor::new("lorem");
        let mut r = NBReader::new(f, Options::default());
        let bytes = r.read_until(&ReadUntil::NBytes(4)).unwrap();
        assert!(bytes.0.is_empty());
        assert_eq!(bytes.1, "lore");
        assert_eq!(Some('m'), r.try_read());
        assert_eq!(None, r.try_read());
        assert_eq!(None, r.try_read());
        assert_eq!(None, r.try_read());
        assert_eq!(None, r.try_read());
    }
}
