use std::path::PathBuf;

pub use flox_core::traceable_path;

/// Returns a `tracing`-compatible form of an `Option<PathBuf>`
pub fn maybe_traceable_path(maybe_path: &Option<PathBuf>) -> impl tracing::Value {
    if let Some(ref p) = maybe_path {
        p.display().to_string()
    } else {
        String::from("null")
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use std::fmt::Display;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, Default)]
    pub struct CollectingWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl Display for CollectingWriter {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let buffer = self.buffer.lock().unwrap();
            let str_content = String::from_utf8_lossy(&buffer);
            write!(f, "{str_content}")
        }
    }

    impl std::io::Write for &CollectingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // panic!("Cannot write to a read-only writer");
            self.buffer.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.buffer.lock().unwrap().flush()
        }
    }

    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for CollectingWriter {
        type Writer = <Mutex<Vec<u8>> as tracing_subscriber::fmt::MakeWriter<'w>>::Writer;

        fn make_writer(&'w self) -> Self::Writer {
            (*self.buffer).make_writer()
        }
    }

    // For now this is a POC of using tracing for output tests,
    // evenatually we should probably move that to the tracing utils or `message` module.
    #[cfg(any(test, feature = "tests"))]
    pub fn test_subscriber() -> (impl tracing::Subscriber, CollectingWriter) {
        let writer = CollectingWriter::default();

        // TODO: also tee to test output?
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .compact()
            .without_time()
            .with_level(false)
            .with_target(false)
            .finish();

        (subscriber, writer)
    }
}
