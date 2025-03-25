use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Write;

use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::composite::{Warning, COMPOSER_MANIFEST_ID};
use flox_rust_sdk::models::manifest::raw::PackageToInstall;
use indoc::formatdoc;
use tracing::info;

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

    info!("{v}");
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
pub(crate) fn info(v: impl Display) {
    print_message(std::format_args!("ℹ️ {v}"));
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

/// Display a message for packages that were successfully installed for all
/// requested systems.
pub(crate) fn packages_successfully_installed(
    pkgs: &[PackageToInstall],
    environment_description: &str,
) {
    if !pkgs.is_empty() {
        let pkg_list = pkgs
            .iter()
            .map(|p| format!("'{}'", p.id()))
            .collect::<Vec<_>>()
            .join(", ");
        updated(format!(
            "{pkg_list} installed to environment {environment_description}"
        ));
    }
}

/// Display messages for each package that could only be installed for some of
/// the requested systems.
pub(crate) fn packages_installed_with_system_subsets(pkgs: &[PackageToInstall]) {
    for pkg in pkgs.iter() {
        warning(format!(
            "'{}' installed only for the following systems: {}",
            pkg.id(),
            // Only `None` for flakes, which can't reach this code
            // path anyway.
            pkg.systems().unwrap_or_default().join(", ")
        ))
    }
}

/// Display a message for packages that were requested but were already installed.
pub(crate) fn packages_already_installed(pkgs: &[PackageToInstall], environment_description: &str) {
    let already_installed_msg = match pkgs {
        [] => None,
        [pkg] => Some(format!(
            "Package with id '{}' already installed to environment {environment_description}",
            pkg.id()
        )),
        pkgs => {
            let joined = pkgs
                .iter()
                .map(|p| format!("'{}'", p.id()))
                .collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!("Packages with ids {joined} already installed to environment {environment_description}"))
        },
    };
    if let Some(msg) = already_installed_msg {
        warning(msg)
    }
}

/// Format a list of overridden fields for an environment.
fn format_overridden_fields(fields: &[String]) -> String {
    fields
        .iter()
        .map(|key| format!("  - {}", key))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print notices for any environments that have overridden fields during composition.
pub(crate) fn print_overridden_manifest_fields(lockfile: &Lockfile) {
    let Some(ref compose) = lockfile.compose else {
        return;
    };

    type Field = String;
    type Environment = String;

    // De-duplicate fields by the last "winning" environment.
    let winning_env_by_field: BTreeMap<Field, Environment> = compose
        .warnings
        .iter()
        .filter_map(|warning_context| match &warning_context.warning {
            Warning::Overriding(field) => Some((
                field.to_string(),
                warning_context.higher_priority_name.clone(),
            )),
            _ => None,
        })
        .collect();

    // Invert the de-duplicated map.
    let mut fields_by_env: BTreeMap<Environment, Vec<Field>> = BTreeMap::new();
    for (field, env) in winning_env_by_field {
        fields_by_env.entry(env).or_default().push(field);
    }

    // Sort the notices by the order that the environments were included and
    // then the current composer environment (if present) last.
    let mut messages_by_env: Vec<String> = Vec::new();
    let ordered_envs = compose.include.iter().map(|include| include.name.clone());
    for env in ordered_envs {
        if let Some(fields) = fields_by_env.get(&env) {
            messages_by_env.push(format!(
                "- Environment '{}' set:\n{}",
                env,
                format_overridden_fields(fields),
            ));
        }
    }
    if let Some(fields) = fields_by_env.get(COMPOSER_MANIFEST_ID) {
        messages_by_env.push(format!(
            "- This environment set:\n{}",
            format_overridden_fields(fields),
        ));
    }
    if !messages_by_env.is_empty() {
        let message = formatdoc! {"
                The following manifest fields were overridden during merging:
                {}", messages_by_env.join("\n")
        };
        info(message);
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;

    #[tokio::test]
    async fn test_print_overridden_manifest_fields() {
        let (mut flox, _tempdir) = flox_instance();
        flox.features.compose = true;

        let mut dep1 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "set by dep1"
            overridden_by_dep2 = "set by dep1"
            overridden_by_composer = "set by dep1"
        "#});
        dep1.lockfile(&flox).unwrap();

        let mut dep2 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by dep2"
            overridden_by_dep2 = "updated by dep2"
        "#});
        dep2.lockfile(&flox).unwrap();

        let composer_original_manifest = formatdoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by composer"
            overridden_by_composer = "updated by composer"

            [include]
            environments = [
                {{ dir = "{dep1_dir}", name = "dep_one" }},
                {{ dir = "{dep2_dir}", name = "dep_two" }},
            ]"#,
            dep1_dir = dep1.parent_path().unwrap().to_string_lossy(),
            dep2_dir = dep2.parent_path().unwrap().to_string_lossy(),
        };
        let mut composer = new_path_environment(&flox, &composer_original_manifest);
        let lockfile = composer.lockfile(&flox).unwrap();

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_overridden_manifest_fields(&lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        // - environmemnts are listed by the order they were included
        // - composer environment is listed last
        // - environment `dep_one` doesn't appear because its fields are overridden later
        assert_eq!(writer.to_string(), indoc! {"
            ℹ️ The following manifest fields were overridden during merging:
            - Environment 'dep_two' set:
              - vars.overridden_by_dep2
            - This environment set:
              - vars.overridden_by_all
              - vars.overridden_by_composer
            "});
    }
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
