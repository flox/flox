use std::fmt::Display;
use std::io::Write;

use flox_rust_sdk::models::manifest::PackageToInstall;
/// Write a message to stderr.
///
/// This is a wrapper around `eprintln!` that can be further extended
/// to include logging, word wrapping, ANSI filtereing etc.
fn print_message(v: impl Display) {
    #[cfg(test)]
    {
        let history = crate::utils::message::history::History::global();
        history.push_message(format!("{v}"));
    }

    eprintln!("{v}");
}

fn print_message_to_buffer(out: &mut impl Write, v: impl Display) {
    writeln!(out, "{v}").unwrap();
}

/// alias for [print_message]
pub(crate) fn plain(v: impl Display) {
    print_message(v);
}
pub(crate) fn error(v: impl Display) {
    print_message(std::format_args!("❌ ERROR: {v}"));
}
pub(crate) fn created(v: impl Display) {
    print_message(std::format_args!("✨ {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn deleted(v: impl Display) {
    print_message(std::format_args!("🗑️  {v}"));
}
pub(crate) fn updated(v: impl Display) {
    print_message(std::format_args!("✅ {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn warning(v: impl Display) {
    print_message(std::format_args!("⚠️  {v}"));
}

/// double width character, add an additional space for alignment
pub(crate) fn warning_to_buffer(out: &mut impl Write, v: impl Display) {
    print_message_to_buffer(out, std::format_args!("⚠️  {v}"));
}

pub(crate) fn package_installed(pkg: &PackageToInstall, environment_description: &str) {
    updated(format!(
        "'{}' installed to environment {environment_description}",
        pkg.id()
    ));
}

/// A history for messages printed to stderr through the `message` module .
/// In unit tests, the messaging functions of the `message` module will,
/// populate a `History` in addition to printing the message to stderr.
/// This allows the test to assert against the messages printed to stderr,
/// without refactoring existing implementations to print to a `Write` trait
/// and injecting a mock in tests.
///
/// The `History` is thus comparitivly non-intrusive.
/// However, it comes with a few caveats:
///
/// Messaging functions are currently effectively (process) globals,
/// inheriting the global nature of `stderr`.
/// `#[test]` functions are run in parallel on separate _threads_
/// by both `cargo test` and `cargo nextest`.
/// Assuming an equally global `History`, messages from one test
/// will be visible in another test.
/// There are several approaches to avoid this:
///
/// * running tests in sequence (e.g. `cargo test -- --test-threads=1`),
///   slows down the test suite.
/// * additional bookkeeping of scopes (e.g. via `tracing`),
///   complicates the test setup, wither needs to be done manually,
///   or we maintain a `#[test]` replacement. (That said,
///   there may be something to learn from `tracing`s architecture).
/// * Pass a `History` instance to the messaging functions,
///   intrusive as it reqeuries all callers to be updated.
/// * Use a [thread_local!] `History`, which is the current approach.
///
/// Being thread local, the `History` is not shared between threads,
/// i.e. not shared between tests.
/// On the flip side, if the test spawns additional threads,
/// which print messages, these will be captured in the `History`
/// of the thread that spawned them, rather than the test's `History`.
/// Likewise if the test uses a multi-threaded async runtime, like `tokio`,
/// messages from spawned tasks will not be captured in the `History`.
/// Also due to runtimes sharing threads between tasks, the `History` available
/// in the task is also shared between tasks.
///
/// Luckily, our CLI surface is largely single threaded.
/// Threads are mainly spawned for blocking operations either in the library layer,
/// or to run library functions in the background, e.g. to print a progress bar
/// on the main thread.
/// Also, while technically within an async runtime, the CLI is largely synchronous,
/// and the default [#[tokio::test]] runtime is single threaded.
#[cfg(test)]
pub mod history {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    thread_local! {
        static THREAD_HISTORY: Rc<RefCell<VecDeque<String>>> = {
            Rc::new(RefCell::new(VecDeque::new()))
        };
    }

    pub(crate) struct History {
        messages: Rc<RefCell<VecDeque<String>>>,
    }

    impl History {
        pub(crate) fn global() -> History {
            let messages = THREAD_HISTORY.with(|h| h.clone());
            History { messages }
        }

        /// Get a snapshot of the messages at the time of the call.
        ///
        /// We use this over an iterator to avoid accidental panics (from [RefCell::borrow_mut]),
        /// if messages are added or cleared before the iterator is dropped.
        ///
        /// Messages are returned in the order they were printed,
        /// i.e. ordered oldest to newest.
        pub(crate) fn messages(&self) -> VecDeque<String> {
            self.messages.borrow().clone()
        }

        /// Push a message to the history of the current thread.
        /// This is currently automatically called by the messaging functions
        /// that call [super::print_message].
        pub(crate) fn push_message(&self, message: String) {
            self.messages.borrow_mut().push_back(message);
        }

        /// Clear the history of the current thread.
        pub(crate) fn clear(&self) {
            self.messages.borrow_mut().clear();
        }
    }

    /// Tests that show the behavior of the `History` in a concurrent environments.
    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::utils::message::plain;

        #[test]
        fn concurrent_1() {
            plain("1");
            assert_eq!(&History::global().messages(), &["1"])
        }

        #[test]
        fn concurrent_2() {
            plain("2");
            assert_eq!(&History::global().messages(), &["2"])
        }

        async fn check_and_produce_inner(expect: &[&str]) {
            assert_eq!(&History::global().messages(), &expect);
            plain("async");
        }

        #[tokio::test]
        async fn concurrent_async_3() {
            plain("3");
            check_and_produce_inner(&["3"]).await;
            assert_eq!(&History::global().messages(), &["3", "async"])
        }

        #[tokio::test]
        async fn concurrent_async_4() {
            plain("4");
            check_and_produce_inner(&["4"]).await;
            assert_eq!(&History::global().messages(), &["4", "async"])
        }

        /// In a "multi_thread" tokio runtime,
        /// spawned tasks are run on separate threads,
        /// [tokio::spawn] will spawn the futures on individual threads,
        /// to allow them to be worked in parallel.
        /// Since these futures are not worked on the main thread,
        /// the history will not be shared and cannot be asserted against.
        ///
        /// In addition, tokio will _reuse_ threads,
        /// so messages from other tasks may appear in the history.
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn parallel_async() {
            let fut_1 = tokio::spawn(async {
                plain("1");
            });
            let fut_2 = tokio::spawn(async {
                plain("2");
            });
            let fut_3 = tokio::spawn(async {
                plain("3");
            });
            let fut_4 = tokio::spawn(async {
                plain("4");
            });
            let _ = tokio::join!(fut_1, fut_2, fut_3, fut_4);

            let messages = History::global().messages();
            assert_eq!(messages.len(), 0, "Messages: {messages:?}",);
        }

        /// This test will fail because the history is thread specific.
        /// Each thread has its own history,
        /// so the main thread won't see the messages from the other threads.
        #[test]
        fn parallel() {
            std::thread::scope(|scope| {
                scope.spawn(|| plain("1"));
                scope.spawn(|| plain("2"));
                scope.spawn(|| plain("3"));
                scope.spawn(|| plain("4"));
            });

            assert_eq!(History::global().messages().len(), 0)
        }

        #[test]
        fn clear() {
            let history = History::global();

            plain("message");
            assert_eq!(&history.messages(), &["message"]);
            history.clear();
            assert_eq!(history.messages().len(), 0);
        }
    }
}
