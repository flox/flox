//! Collection-aware Secret Service access. Discovery never unlocks anything.
//!
//! `keyring-core` 1.0's `Entry::new` supplies no target. The old zbus store
//! searches by `service` and `username` across all collections, and creates in
//! the default alias. An explicit target such as `Login` is a case-sensitive
//! collection *label* and an additional item attribute, not the `login` alias.
//! Keep those attributes searchable, including when an item has been moved.

use std::collections::HashMap;
use std::env;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_io::Timer;
use futures::StreamExt;
use futures::future::{Either, select};
use indoc::formatdoc;
use inquire::{Password, PasswordDisplayMode};
use thiserror::Error;
use zbus::address::Transport;
use zbus::proxy::{Builder, CacheProperties};
use zbus::zvariant::{Dict, OwnedObjectPath, OwnedValue, Value};
use zbus::{Address, Connection, Proxy, connection};
use zeroize::Zeroizing;

use super::CredentialStoreError;
use crate::utils::dialog::{Checkpoint, Dialog, WaitResult, flox_theme};
use crate::utils::{TERMINAL_STDERR, message};

const SERVICE: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const GNOME_INTERFACE: &str = "org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(30);

// Startup resolution and command handlers can access separate store instances.
// Do not ask again after cancellation, including during later migration.
static UNLOCK_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Backend errors deliberately retain neither D-Bus bodies nor provider error
/// messages: a provider can include secret material in an error reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Error {
    #[error("Could not access the system keyring.\nCheck that its service is running.")]
    Backend,
    #[error("Keyring access requires a local Unix session bus.\nUse a local desktop session.")]
    UnsupportedTransport,
    #[error("The system keyring is locked.\nUnlock it and retry the command.")]
    Locked,
    #[error("Keyring unlocking was cancelled.\nRetry the command to unlock it.")]
    Cancelled,
    #[error("The keyring did not respond in time.\nUnlock it and retry the command.")]
    TimedOut,
    #[error(
        "Terminal unlocking is unavailable for this keyring.\nUse its desktop app to unlock it."
    )]
    Unsupported,
    #[error("The keyring password was incorrect.\nRetry the command to try again.")]
    IncorrectPassword,
    #[error("No default keyring is configured.\nChoose a default in your keyring app.")]
    NoDefault,
    #[error(
        "Multiple matching credentials exist.\nRemove duplicate Flox entries in your keyring app."
    )]
    Ambiguous,
}

impl From<zbus::Error> for Error {
    fn from(_: zbus::Error) -> Self {
        Self::Backend
    }
}

async fn bounded<T>(future: impl Future<Output = T>, duration: Duration) -> Result<T, Error> {
    match select(Box::pin(future), Box::pin(Timer::after(duration))).await {
        Either::Left((result, _)) => Ok(result),
        Either::Right(_) => Err(Error::TimedOut),
    }
}

/// Environment hints choose the initial unlock method, not whether prompting
/// is allowed. A present display can still be unusable, so desktop unlocking
/// must also support an immediate terminal switch and a bounded fallback.
fn has_desktop_session() -> bool {
    let has_value = |name| env::var_os(name).is_some_and(|value| !value.is_empty());
    !has_value("SSH_TTY")
        && !has_value("SSH_CONNECTION")
        && (has_value("DISPLAY") || has_value("WAYLAND_DISPLAY"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnlockPromptOutcome {
    Completed,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Collection {
    path: OwnedObjectPath,
    label: String,
    items: Vec<OwnedObjectPath>,
}

/// Prefer an existing Login credential, then the default collection, then a
/// unique existing credential elsewhere. Empty collections have no priority.
/// Never interpret a failed search or an ambiguity as permission to create.
fn existing_collection(
    collections: &[Collection],
    default: &OwnedObjectPath,
) -> Result<Option<usize>, Error> {
    let rank = |collection: &Collection| {
        if collection.label == "Login" {
            0
        } else if &collection.path == default {
            1
        } else {
            2
        }
    };
    let best = collections
        .iter()
        .filter(|collection| !collection.items.is_empty())
        .map(rank)
        .min();
    let Some(best) = best else {
        return Ok(None);
    };
    let mut matches = collections
        .iter()
        .enumerate()
        .filter(|(_, collection)| !collection.items.is_empty() && rank(collection) == best);
    let (index, collection) = matches.next().expect("a minimum has a matching collection");
    if collection.items.len() != 1 || matches.next().is_some() {
        return Err(Error::Ambiguous);
    }
    Ok(Some(index))
}

// No Debug implementation: a write operation contains a bearer credential.
enum Operation<'a> {
    Get,
    Set(&'a str),
    Remove,
}

pub(super) fn get(
    service: &str,
    account: &str,
    allow_prompt: bool,
) -> Result<Option<String>, CredentialStoreError> {
    run(service, account, Operation::Get, allow_prompt)
}

pub(super) fn set(
    service: &str,
    account: &str,
    token: &str,
    allow_prompt: bool,
) -> Result<(), CredentialStoreError> {
    run(service, account, Operation::Set(token), allow_prompt).map(|_| ())
}

pub(super) fn remove(
    service: &str,
    account: &str,
    allow_prompt: bool,
) -> Result<(), CredentialStoreError> {
    match run(service, account, Operation::Remove, allow_prompt) {
        // A missing service is harmless on logout. Failures after connecting
        // are surfaced, so a locked credential is never reported as deleted.
        Ok(_) | Err(CredentialStoreError::NoBackend(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

fn run(
    service: &str,
    account: &str,
    operation: Operation<'_>,
    allow_prompt: bool,
) -> Result<Option<String>, CredentialStoreError> {
    pollster::block_on(async {
        let address = Address::session().map_err(|_| Error::Backend)?;
        let builder = local_connection(address)?;
        let connection = bounded(builder.build(), CALL_TIMEOUT)
            .await
            .map_err(CredentialStoreError::from)?
            .map_err(|_| CredentialStoreError::NoBackend(keyring_core::Error::NoDefaultStore))?;
        let result = async {
            let mut client = Client::connect(connection.clone()).await?;
            client.can_prompt &= allow_prompt;
            client
                .perform(service, account, operation)
                .await
                .map_err(Into::into)
        }
        .await;
        // A dedicated connection scopes sessions and pending prompts to this
        // operation, including timeouts and provider failures.
        let _ = bounded(connection.close(), CALL_TIMEOUT).await;
        result
    })
}

fn local_connection(address: Address) -> Result<connection::Builder<'static>, Error> {
    if !matches!(address.transport(), Transport::Unix(_)) {
        return Err(Error::UnsupportedTransport);
    }
    Ok(connection::Builder::address(address)?.method_timeout(CALL_TIMEOUT))
}

#[derive(Debug, Clone)]
struct Client {
    connection: Connection,
    owner: String,
    session: OwnedObjectPath,
    can_prompt: bool,
    prompt_timeout: Duration,
}

impl Client {
    async fn connect(connection: Connection) -> Result<Self, CredentialStoreError> {
        // The standard plain session travels only over the user's authenticated
        // local session bus. The provider owns encryption at rest. Neither the
        // master password nor credentials are passed to a child process.
        let reply = bounded(
            connection.call_method(
                Some(SERVICE),
                ROOT,
                Some(SERVICE_INTERFACE),
                "OpenSession",
                &("plain", Value::from("")),
            ),
            CALL_TIMEOUT,
        )
        .await?;
        let reply = match reply {
            Ok(reply) => reply,
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.ServiceUnknown"
                        | "org.freedesktop.DBus.Error.NameHasNoOwner"
                ) =>
            {
                return Err(CredentialStoreError::NoBackend(
                    keyring_core::Error::NoDefaultStore,
                ));
            },
            Err(_) => return Err(Error::Backend.into()),
        };
        // Pin subsequent calls to the provider that created this session.
        let owner = reply.header().sender().ok_or(Error::Backend)?.to_string();
        let (_, session): (OwnedValue, OwnedObjectPath) =
            reply.body().deserialize().map_err(|_| Error::Backend)?;
        Ok(Self {
            connection,
            owner,
            session,
            can_prompt: Dialog::can_prompt(),
            prompt_timeout: PROMPT_TIMEOUT,
        })
    }

    async fn proxy(
        &self,
        path: OwnedObjectPath,
        interface: &'static str,
    ) -> Result<Proxy<'static>, Error> {
        bounded(
            Builder::<Proxy<'static>>::new(&self.connection)
                .destination(self.owner.clone())?
                .path(path)?
                .interface(interface)?
                .cache_properties(CacheProperties::No)
                .build(),
            CALL_TIMEOUT,
        )
        .await?
        .map_err(Into::into)
    }

    async fn service(&self) -> Result<Proxy<'static>, Error> {
        self.proxy(
            OwnedObjectPath::try_from(ROOT).expect("valid service path"),
            SERVICE_INTERFACE,
        )
        .await
    }

    async fn discover(
        &self,
        attributes: &HashMap<&str, &str>,
    ) -> Result<(Vec<Collection>, OwnedObjectPath), Error> {
        let service = self.service().await?;
        let default: OwnedObjectPath =
            bounded(service.call("ReadAlias", &("default",)), CALL_TIMEOUT).await??;
        let paths: Vec<OwnedObjectPath> =
            bounded(service.get_property("Collections"), CALL_TIMEOUT).await??;
        let mut collections = Vec::new();
        for path in paths {
            let proxy = self.proxy(path.clone(), COLLECTION_INTERFACE).await?;
            let items = bounded(proxy.call("SearchItems", &(attributes,)), CALL_TIMEOUT).await??;
            let label = bounded(proxy.get_property("Label"), CALL_TIMEOUT).await??;
            collections.push(Collection { path, label, items });
        }
        Ok((collections, default))
    }

    async fn perform(
        &self,
        service: &str,
        account: &str,
        operation: Operation<'_>,
    ) -> Result<Option<String>, Error> {
        // Match the old backend's attributes, including items with a target
        // attribute. Adding target=default would hide existing credentials.
        let attributes = HashMap::from([("service", service), ("username", account)]);
        let (collections, default) = self.discover(&attributes).await?;
        if matches!(operation, Operation::Remove) {
            // Unlock all relevant collections before deleting any item. Logout
            // removes older copies too, so a stale token cannot reappear later.
            for collection in collections.iter().filter(|c| !c.items.is_empty()) {
                self.ensure_unlocked(collection).await?;
            }
            for path in collections.iter().flat_map(|c| &c.items) {
                let item = self.proxy(path.clone(), ITEM_INTERFACE).await?;
                let prompt = bounded(item.call("Delete", &()), CALL_TIMEOUT).await??;
                self.complete_prompt(prompt).await?;
            }
            return Ok(None);
        }

        let existing = existing_collection(&collections, &default)?;
        let selected = match (existing, &operation) {
            (Some(index), _) => &collections[index],
            (None, Operation::Get) => return Ok(None),
            (None, _) => collections
                .iter()
                .find(|c| c.path == default)
                .ok_or(Error::NoDefault)?,
        };
        self.ensure_unlocked(selected).await?;
        if let Some(path) = selected.items.first() {
            let item = self.proxy(path.clone(), ITEM_INTERFACE).await?;
            match operation {
                Operation::Get => {
                    let (_, _, bytes, _): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
                        bounded(item.call("GetSecret", &(&self.session,)), CALL_TIMEOUT).await??;
                    return String::from_utf8(bytes)
                        .map(Some)
                        .map_err(|_| Error::Backend);
                },
                Operation::Set(token) => {
                    let secret = (&self.session, &[] as &[u8], token.as_bytes(), "text/plain");
                    bounded(item.call::<_, _, ()>("SetSecret", &(secret,)), CALL_TIMEOUT).await??;
                },
                Operation::Remove => unreachable!(),
            }
            return Ok(None);
        }

        let Operation::Set(token) = operation else {
            unreachable!()
        };
        let collection = self
            .proxy(selected.path.clone(), COLLECTION_INTERFACE)
            .await?;
        let label = format!("keyring:{account}@{service}");
        let properties = HashMap::from([
            (
                "org.freedesktop.Secret.Item.Label",
                Value::from(label.as_str()),
            ),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(Dict::from(attributes)),
            ),
        ]);
        let secret = (&self.session, &[] as &[u8], token.as_bytes(), "text/plain");
        let (item, prompt): (OwnedObjectPath, OwnedObjectPath) = bounded(
            collection.call("CreateItem", &(properties, secret, true)),
            CALL_TIMEOUT,
        )
        .await??;
        let result = self.complete_prompt(prompt).await?;
        if item.as_str() == "/"
            && result
                .and_then(|value| OwnedObjectPath::try_from(value).ok())
                .is_none_or(|path| path.as_str() == "/")
        {
            return Err(Error::Backend);
        }
        Ok(None)
    }

    async fn is_locked(&self, collection: &Collection) -> Result<bool, Error> {
        let proxy = self
            .proxy(collection.path.clone(), COLLECTION_INTERFACE)
            .await?;
        Ok(bounded(proxy.get_property("Locked"), CALL_TIMEOUT).await??)
    }

    async fn ensure_unlocked(&self, collection: &Collection) -> Result<(), Error> {
        if !self.is_locked(collection).await? {
            return Ok(());
        }
        if !self.can_prompt {
            return Err(Error::Locked);
        }
        if UNLOCK_CANCELLED.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let result = self.unlock_interactively(collection).await;
        if let Err(error) = result {
            UNLOCK_CANCELLED.store(true, Ordering::Relaxed);
            if error != Error::Cancelled {
                message::warning(error.to_string());
            }
        }
        result
    }

    async fn unlock_interactively(&self, collection: &Collection) -> Result<(), Error> {
        let terminal_supported = self.supports_terminal_unlock().await;
        if !has_desktop_session() {
            if !terminal_supported {
                return Err(Error::Unsupported);
            }
            message::plain(format!(
                "Keyring {:?} is locked. Unlock it in this terminal.",
                collection.label
            ));
            return self.unlock_with_password(collection).await;
        }

        let result = self
            .unlock_with_desktop(collection, terminal_supported)
            .await;
        match result {
            Ok(UnlockPromptOutcome::Completed) => return Ok(()),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) if !terminal_supported => return Err(error),
            Ok(UnlockPromptOutcome::Terminal) | Err(_) => {},
        }
        // The desktop may have unlocked the collection as the user switched
        // methods or the deadline expired. Avoid asking for a password again.
        if !self.is_locked(collection).await? {
            return Ok(());
        }
        message::plain(match result {
            Err(Error::TimedOut) => {
                "The desktop unlock prompt timed out. Unlock the keyring in this terminal."
            },
            Err(_) => {
                "The desktop prompt could not unlock the keyring. Unlock it in this terminal."
            },
            _ => "Unlock the keyring in this terminal.",
        });
        self.unlock_with_password(collection).await
    }

    async fn unlock_with_desktop(
        &self,
        collection: &Collection,
        terminal_supported: bool,
    ) -> Result<UnlockPromptOutcome, Error> {
        let service = self.service().await?;
        let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = bounded(
            service.call("Unlock", &(vec![&collection.path],)),
            CALL_TIMEOUT,
        )
        .await??;
        if prompt.as_str() != "/" && terminal_supported {
            let message = formatdoc! {"
                Unlock keyring {label:?} in the desktop prompt.
                Press Enter to use the terminal instead, or Ctrl-C to cancel.",
                label = collection.label,
            };
            let terminal_action = Dialog {
                message: &message,
                help_message: None,
                typed: Checkpoint,
            }
            .checkpoint_async();
            if self.complete_unlock_prompt(prompt, terminal_action).await?
                == UnlockPromptOutcome::Terminal
            {
                return Ok(UnlockPromptOutcome::Terminal);
            }
        } else {
            self.complete_prompt(prompt).await?;
        }
        if self.is_locked(collection).await? {
            return Err(Error::Locked);
        }
        Ok(UnlockPromptOutcome::Completed)
    }

    /// Race the desktop prompt against terminal input. Drop the losing future
    /// before another prompt can start, restoring terminal mode and dismissing
    /// the desktop prompt when the user switches methods or cancels.
    async fn complete_unlock_prompt(
        &self,
        path: OwnedObjectPath,
        terminal_action: impl Future<Output = WaitResult>,
    ) -> Result<UnlockPromptOutcome, Error> {
        match select(
            Box::pin(self.complete_prompt(path.clone())),
            Box::pin(terminal_action),
        )
        .await
        {
            Either::Left((result, terminal_action)) => {
                drop(terminal_action);
                // GNOME reports prompter startup failures and user dismissal
                // through the same Completed(true) signal. Both need a terminal
                // fallback; only an explicit terminal interruption cancels here.
                match result {
                    Err(Error::Cancelled) => Ok(UnlockPromptOutcome::Terminal),
                    result => result.map(|_| UnlockPromptOutcome::Completed),
                }
            },
            Either::Right((action, desktop_prompt)) => {
                drop(desktop_prompt);
                if let Ok(proxy) = self.proxy(path, PROMPT_INTERFACE).await {
                    let _ = bounded(proxy.call::<_, _, ()>("Dismiss", &()), CALL_TIMEOUT).await;
                }
                match action {
                    WaitResult::Enter => Ok(UnlockPromptOutcome::Terminal),
                    WaitResult::Interrupted => Err(Error::Cancelled),
                }
            },
        }
    }

    /// The extension is provider-specific and unsupported by GNOME. Capability
    /// discovery controls terminal availability; typed method errors remain authoritative.
    async fn supports_terminal_unlock(&self) -> bool {
        let Ok(proxy) = self
            .proxy(
                OwnedObjectPath::try_from(ROOT).expect("valid service path"),
                "org.freedesktop.DBus.Introspectable",
            )
            .await
        else {
            return false;
        };
        let Ok(Ok(xml)) =
            bounded(proxy.call::<_, _, String>("Introspect", &()), CALL_TIMEOUT).await
        else {
            return false;
        };
        xml.contains(GNOME_INTERFACE) && xml.contains("UnlockWithMasterPassword")
    }

    async fn unlock_with_password(&self, collection: &Collection) -> Result<(), Error> {
        for attempt in 0..3 {
            let password = {
                let _stderr_lock = TERMINAL_STDERR.lock();
                Zeroizing::new(
                    Password::new("Keyring password:")
                        .with_display_mode(PasswordDisplayMode::Hidden)
                        .without_confirmation()
                        .with_help_message("Input is hidden. Esc cancels.")
                        .with_render_config(flox_theme())
                        .prompt()
                        .map_err(|_| Error::Cancelled)?,
                )
            };
            let result = self
                .unlock_with_master_password(collection, &password)
                .await;
            drop(password);
            match result {
                Ok(()) if !self.is_locked(collection).await? => return Ok(()),
                Ok(()) => return Err(Error::Locked),
                Err(Error::IncorrectPassword) if attempt < 2 => {
                    message::warning(
                        "The keyring password was incorrect. Try again or press Esc to cancel.",
                    );
                },
                Err(error) => return Err(error),
            }
        }
        Err(Error::IncorrectPassword)
    }

    async fn unlock_with_master_password(
        &self,
        collection: &Collection,
        password: &str,
    ) -> Result<(), Error> {
        let proxy = self
            .proxy(
                OwnedObjectPath::try_from(ROOT).expect("valid service path"),
                GNOME_INTERFACE,
            )
            .await?;
        // OpenSession and this method use the same connection and provider.
        // Borrow the zeroizing input and never attach it to tracing or errors.
        let secret = (
            &self.session,
            &[] as &[u8],
            password.as_bytes(),
            "text/plain",
        );
        bounded(
            proxy.call::<_, _, ()>("UnlockWithMasterPassword", &(&collection.path, secret)),
            CALL_TIMEOUT,
        )
        .await?
        .map_err(classify_unlock_error)
    }

    /// Subscribe before showing a prompt so an immediate Completed signal is
    /// not missed. Never show one in a pipeline or noninteractive invocation.
    async fn complete_prompt(&self, path: OwnedObjectPath) -> Result<Option<OwnedValue>, Error> {
        if path.as_str() == "/" {
            return Ok(None);
        }
        let proxy = self.proxy(path, PROMPT_INTERFACE).await?;
        let result = bounded(
            async {
                if !self.can_prompt || UNLOCK_CANCELLED.load(Ordering::Relaxed) {
                    return Err(Error::Locked);
                }
                let mut completed =
                    bounded(proxy.receive_signal("Completed"), CALL_TIMEOUT).await??;
                bounded(proxy.call::<_, _, ()>("Prompt", &("",)), CALL_TIMEOUT).await??;
                let signal = completed.next().await.ok_or(Error::Backend)?;
                let (dismissed, value): (bool, OwnedValue) =
                    signal.body().deserialize().map_err(|_| Error::Backend)?;
                if dismissed {
                    return Err(Error::Cancelled);
                }
                Ok(Some(value))
            },
            self.prompt_timeout,
        )
        .await
        .and_then(|result| result);
        if result.is_err() {
            let _ = bounded(proxy.call::<_, _, ()>("Dismiss", &()), CALL_TIMEOUT).await;
        }
        result
    }
}

fn classify_unlock_error(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.gnome.keyring.Error.Denied" => Error::IncorrectPassword,
            "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.NotSupported" => Error::Unsupported,
            _ => Error::Backend,
        },
        _ => Error::Backend,
    }
}

#[cfg(test)]
mod tests;
