use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use pretty_assertions::assert_eq;
use serde::{Deserialize, Serialize};
use zbus::connection;
use zbus::zvariant::Type;

use super::*;

const DEFAULT: &str = "/collection/preferred";
const LEGACY: &str = "/collection/custom_login";
const UNRELATED: &str = "/collection/unrelated";
const SESSION: &str = "/session/test";
const TEST_SERVICE: &str = "dev.flox.flox";
const ACCOUNT: &str = "https://hub.example.test/";

fn path(value: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(value).unwrap()
}

fn collection(name: &str, label: &str, count: usize) -> Collection {
    Collection {
        path: path(name),
        label: label.to_string(),
        items: (0..count)
            .map(|n| path(&format!("{name}/item{n}")))
            .collect(),
    }
}

#[test]
fn selection_preserves_existing_credentials() {
    let cases = [
        // A Login label is independent of its object path and any login alias.
        (
            vec![
                collection(DEFAULT, "Personal", 1),
                collection(LEGACY, "Login", 1),
            ],
            Ok(Some(1)),
        ),
        // An empty or differently capitalized label has no legacy priority.
        (
            vec![
                collection(LEGACY, "Login", 0),
                collection(DEFAULT, "Personal", 1),
            ],
            Ok(Some(1)),
        ),
        (
            vec![
                collection(LEGACY, "login", 1),
                collection(DEFAULT, "Personal", 1),
            ],
            Ok(Some(1)),
        ),
        // Preserve a moved credential instead of creating another in default.
        (
            vec![
                collection(UNRELATED, "Moved", 1),
                collection(DEFAULT, "Personal", 0),
            ],
            Ok(Some(0)),
        ),
        // Empty keyrings do not trigger unlocks while looking for credentials.
        (
            vec![
                collection(LEGACY, "Login", 0),
                collection(DEFAULT, "Personal", 0),
            ],
            Ok(None),
        ),
        // Never guess between duplicate credentials at the selected priority.
        (vec![collection(LEGACY, "Login", 2)], Err(Error::Ambiguous)),
        (
            vec![
                collection(LEGACY, "Login", 1),
                collection(DEFAULT, "Login", 1),
            ],
            Err(Error::Ambiguous),
        ),
        (
            vec![
                collection(LEGACY, "Moved", 1),
                collection(UNRELATED, "Backup", 1),
            ],
            Err(Error::Ambiguous),
        ),
    ];
    for (collections, expected) in cases {
        assert_eq!(existing_collection(&collections, &path(DEFAULT)), expected);
    }
    assert_eq!(
        existing_collection(&[collection(LEGACY, "Login", 1)], &path("/")),
        Ok(Some(0))
    );
}

#[test]
fn plain_sessions_require_a_local_unix_transport() {
    for (address, expected) in [
        ("unix:path=/tmp/flox-keyring-test", Ok(())),
        ("unix:abstract=flox-keyring-test", Ok(())),
        (
            "tcp:host=127.0.0.1,port=12345",
            Err(Error::UnsupportedTransport),
        ),
        ("unixexec:path=/bin/false", Err(Error::UnsupportedTransport)),
    ] {
        assert_eq!(
            local_connection(address.parse().unwrap()).map(|_| ()),
            expected
        );
    }
}

#[test]
fn an_unresponsive_operation_has_a_deadline() {
    assert_eq!(
        pollster::block_on(bounded(futures::future::pending::<()>(), Duration::ZERO)),
        Err(Error::TimedOut),
    );
}

#[test]
fn password_errors_are_classified_without_retaining_provider_text() {
    for (name, expected) in [
        ("org.gnome.keyring.Error.Denied", Error::IncorrectPassword),
        (
            "org.freedesktop.DBus.Error.UnknownMethod",
            Error::Unsupported,
        ),
        (
            "org.freedesktop.DBus.Error.UnknownInterface",
            Error::Unsupported,
        ),
        (
            "org.freedesktop.DBus.Error.NotSupported",
            Error::Unsupported,
        ),
        ("org.freedesktop.DBus.Error.Failed", Error::Backend),
    ] {
        let call = zbus::Message::method_call("/", "UnlockWithMasterPassword")
            .unwrap()
            .build(&())
            .unwrap();
        let reply = zbus::Message::error(&call.header(), name)
            .unwrap()
            .build(&"sensitive-provider-text")
            .unwrap();
        assert_eq!(classify_unlock_error(zbus::Error::from(reply)), expected);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    attributes: HashMap<String, String>,
    token: String,
}

fn stored(token: &str) -> Stored {
    Stored {
        attributes: HashMap::from([
            ("service".into(), TEST_SERVICE.into()),
            ("username".into(), ACCOUNT.into()),
        ]),
        token: token.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    collections: Vec<Collection>,
    locked: Vec<String>,
    items: HashMap<String, Stored>,
    mutations: Vec<String>,
}

#[derive(Debug, Clone)]
struct FakeService(Arc<Mutex<State>>);

#[zbus::interface(name = "org.freedesktop.Secret.Service")]
impl FakeService {
    fn open_session(&self, algorithm: &str, _input: Value<'_>) -> (OwnedValue, OwnedObjectPath) {
        assert_eq!(algorithm, "plain");
        (Value::from("").try_to_owned().unwrap(), path(SESSION))
    }

    fn read_alias(&self, name: &str) -> OwnedObjectPath {
        // Looking up login would select a different collection on real systems.
        assert_eq!(name, "default");
        path(DEFAULT)
    }

    #[zbus(property)]
    fn collections(&self) -> Vec<OwnedObjectPath> {
        self.0
            .lock()
            .unwrap()
            .collections
            .iter()
            .map(|c| c.path.clone())
            .collect()
    }

    fn unlock(&self, _objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        panic!("noninteractive discovery and access must never request unlocking")
    }
}

#[derive(Debug, Clone)]
struct FakeCollection {
    path: String,
    state: Arc<Mutex<State>>,
}

// A single returned Secret is a struct-valued D-Bus argument.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
struct Secret {
    session: OwnedObjectPath,
    parameters: Vec<u8>,
    value: Vec<u8>,
    content_type: String,
}

#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl FakeCollection {
    #[zbus(property)]
    fn label(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .collections
            .iter()
            .find(|c| c.path.as_str() == self.path)
            .unwrap()
            .label
            .clone()
    }

    #[zbus(property)]
    fn locked(&self) -> bool {
        self.state.lock().unwrap().locked.contains(&self.path)
    }

    fn search_items(&self, attributes: HashMap<String, String>) -> Vec<OwnedObjectPath> {
        let state = self.state.lock().unwrap();
        state
            .items
            .iter()
            .filter(|(item_path, item)| {
                item_path.starts_with(&format!("{}/", self.path))
                    && attributes
                        .iter()
                        .all(|(key, value)| item.attributes.get(key) == Some(value))
            })
            .map(|(item_path, _)| path(item_path))
            .collect()
    }

    fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: Secret,
        replace: bool,
    ) -> (OwnedObjectPath, OwnedObjectPath) {
        assert_eq!(secret.session, path(SESSION));
        assert_eq!(replace, true);
        let attributes = HashMap::<String, String>::try_from(
            properties["org.freedesktop.Secret.Item.Attributes"]
                .try_clone()
                .unwrap(),
        )
        .unwrap();
        let mut state = self.state.lock().unwrap();
        let item_path = format!("{}/item0", self.path);
        state.items.insert(item_path.clone(), Stored {
            attributes,
            token: String::from_utf8(secret.value).unwrap(),
        });
        state.mutations.push(format!("create:{}", self.path));
        (path(&item_path), path("/"))
    }
}

#[derive(Debug, Clone)]
struct FakeItem {
    path: String,
    state: Arc<Mutex<State>>,
}

#[zbus::interface(name = "org.freedesktop.Secret.Item")]
impl FakeItem {
    fn get_secret(&self, session: OwnedObjectPath) -> (Secret,) {
        assert_eq!(session, path(SESSION));
        let token = self.state.lock().unwrap().items[&self.path].token.clone();
        (Secret {
            session,
            parameters: vec![],
            value: token.into_bytes(),
            content_type: "text/plain".into(),
        },)
    }

    fn set_secret(&self, secret: Secret) {
        assert_eq!(secret.session, path(SESSION));
        let mut state = self.state.lock().unwrap();
        state.items.get_mut(&self.path).unwrap().token = String::from_utf8(secret.value).unwrap();
        state.mutations.push(format!("set:{}", self.path));
    }

    fn delete(&self) -> OwnedObjectPath {
        let mut state = self.state.lock().unwrap();
        state.items.remove(&self.path);
        state.mutations.push(format!("delete:{}", self.path));
        path("/")
    }
}

/// Each test uses an authenticated socket pair, with no session bus, daemon,
/// desktop, or access to the developer's keyring.
async fn private_service(state: Arc<Mutex<State>>) -> (Client, Connection) {
    let (server_socket, client_socket) = UnixStream::pair().unwrap();
    let mut server = connection::Builder::unix_stream(server_socket)
        .server(zbus::Guid::generate())
        .unwrap()
        .p2p()
        .unique_name(":1.42")
        .unwrap()
        .serve_at(ROOT, FakeService(state.clone()))
        .unwrap();
    for collection in &state.lock().unwrap().collections {
        server = server
            .serve_at(collection.path.clone(), FakeCollection {
                path: collection.path.to_string(),
                state: state.clone(),
            })
            .unwrap();
        server = server
            .serve_at(path(&format!("{}/item0", collection.path)), FakeItem {
                path: format!("{}/item0", collection.path),
                state: state.clone(),
            })
            .unwrap();
    }
    let (server, connection) = futures::try_join!(
        server.build(),
        connection::Builder::unix_stream(client_socket)
            .p2p()
            .build(),
    )
    .unwrap();
    let mut client = Client::connect(connection).await.unwrap();
    client.interactive = false;
    (client, server)
}

fn initial_state() -> State {
    State {
        collections: vec![
            collection(DEFAULT, "Personal", 0),
            collection(LEGACY, "Login", 0),
            collection(UNRELATED, "Other", 0),
        ],
        locked: vec![UNRELATED.into()],
        items: HashMap::new(),
        mutations: vec![],
    }
}

#[test]
fn new_credentials_use_default_and_do_not_unlock_empty_legacy() {
    pollster::block_on(async {
        let mut initial = initial_state();
        initial.locked.push(LEGACY.into());
        let state = Arc::new(Mutex::new(initial.clone()));
        let (client, _server) = private_service(state.clone()).await;
        assert_eq!(
            client.perform(TEST_SERVICE, ACCOUNT, Operation::Get).await,
            Ok(None)
        );
        assert_eq!(
            client
                .perform(TEST_SERVICE, ACCOUNT, Operation::Set("new-token"))
                .await,
            Ok(None)
        );
        assert_eq!(
            client.perform(TEST_SERVICE, ACCOUNT, Operation::Get).await,
            Ok(Some("new-token".into()))
        );
        initial
            .items
            .insert(format!("{DEFAULT}/item0"), stored("new-token"));
        initial.mutations.push(format!("create:{DEFAULT}"));
        assert_eq!(*state.lock().unwrap(), initial);
    });
}

#[test]
fn legacy_credentials_are_read_updated_and_removed_in_place() {
    pollster::block_on(async {
        let mut initial = initial_state();
        let mut credential = stored("old-token");
        credential
            .attributes
            .insert("target".into(), "Login".into());
        initial.items.insert(format!("{LEGACY}/item0"), credential);
        initial.locked.push(DEFAULT.into());
        let state = Arc::new(Mutex::new(initial.clone()));
        let (client, _server) = private_service(state.clone()).await;
        assert_eq!(
            client.perform(TEST_SERVICE, ACCOUNT, Operation::Get).await,
            Ok(Some("old-token".into()))
        );
        assert_eq!(
            client
                .perform(TEST_SERVICE, ACCOUNT, Operation::Set("updated-token"))
                .await,
            Ok(None)
        );
        assert_eq!(
            client.perform(TEST_SERVICE, ACCOUNT, Operation::Get).await,
            Ok(Some("updated-token".into()))
        );
        assert_eq!(
            client
                .perform(TEST_SERVICE, ACCOUNT, Operation::Remove)
                .await,
            Ok(None)
        );
        initial.items.clear();
        initial.mutations = vec![
            format!("set:{LEGACY}/item0"),
            format!("delete:{LEGACY}/item0"),
        ];
        assert_eq!(*state.lock().unwrap(), initial);
    });
}

#[test]
fn locked_legacy_prevents_fallback_to_default_or_partial_logout() {
    pollster::block_on(async {
        let mut initial = initial_state();
        initial
            .items
            .insert(format!("{LEGACY}/item0"), stored("legacy-token"));
        initial
            .items
            .insert(format!("{DEFAULT}/item0"), stored("default-token"));
        initial.locked.push(LEGACY.into());
        let state = Arc::new(Mutex::new(initial.clone()));
        let (client, _server) = private_service(state.clone()).await;
        for operation in [
            Operation::Get,
            Operation::Set("replacement"),
            Operation::Remove,
        ] {
            assert_eq!(
                client.perform(TEST_SERVICE, ACCOUNT, operation).await,
                Err(Error::Locked)
            );
        }
        assert_eq!(*state.lock().unwrap(), initial);
    });
}

#[test]
fn logout_removes_all_matching_copies_and_preserves_other_accounts() {
    pollster::block_on(async {
        let mut initial = initial_state();
        initial
            .items
            .insert(format!("{LEGACY}/item0"), stored("legacy-token"));
        initial
            .items
            .insert(format!("{DEFAULT}/item0"), stored("default-token"));
        let mut other_account = stored("other-token");
        other_account
            .attributes
            .insert("username".into(), "https://other.example.test/".into());
        initial
            .items
            .insert(format!("{UNRELATED}/item0"), other_account);
        let state = Arc::new(Mutex::new(initial.clone()));
        let (client, _server) = private_service(state.clone()).await;
        assert_eq!(
            client
                .perform(TEST_SERVICE, ACCOUNT, Operation::Remove)
                .await,
            Ok(None)
        );
        assert_eq!(
            client.perform(TEST_SERVICE, ACCOUNT, Operation::Get).await,
            Ok(None)
        );
        initial.items.remove(&format!("{DEFAULT}/item0"));
        initial.items.remove(&format!("{LEGACY}/item0"));
        initial.mutations = vec![
            format!("delete:{DEFAULT}/item0"),
            format!("delete:{LEGACY}/item0"),
        ];
        assert_eq!(*state.lock().unwrap(), initial);
    });
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.gnome.keyring.Error")]
enum GnomeError {
    Denied(String),
}

#[derive(Debug, Clone)]
struct FakeGnome(Arc<Mutex<State>>);

#[zbus::interface(name = "org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface")]
impl FakeGnome {
    fn unlock_with_master_password(
        &self,
        collection: OwnedObjectPath,
        master: Secret,
    ) -> Result<(), GnomeError> {
        assert_eq!(master.session, path(SESSION));
        assert_eq!(master.parameters, Vec::<u8>::new());
        assert_eq!(master.content_type, "text/plain");
        if master.value != b"fixture-password" {
            return Err(GnomeError::Denied("The password was invalid".into()));
        }
        let mut state = self.0.lock().unwrap();
        state.locked.retain(|locked| locked != collection.as_str());
        state.mutations.push(format!("unlock:{collection}"));
        Ok(())
    }
}

#[test]
fn terminal_extension_uses_the_open_session_and_selected_collection() {
    pollster::block_on(async {
        let mut initial = initial_state();
        initial.locked.push(LEGACY.into());
        let state = Arc::new(Mutex::new(initial.clone()));
        let (client, server) = private_service(state.clone()).await;
        let legacy = collection(LEGACY, "Login", 1);
        assert_eq!(client.supports_terminal_unlock().await, false);
        assert_eq!(
            client
                .unlock_with_master_password(&legacy, "fixture-password")
                .await,
            Err(Error::Unsupported)
        );
        server
            .object_server()
            .at(ROOT, FakeGnome(state.clone()))
            .await
            .unwrap();
        assert_eq!(client.supports_terminal_unlock().await, true);
        assert_eq!(
            client
                .unlock_with_master_password(&legacy, "wrong-password")
                .await,
            Err(Error::IncorrectPassword)
        );
        assert_eq!(*state.lock().unwrap(), initial);
        assert_eq!(
            client
                .unlock_with_master_password(&legacy, "fixture-password")
                .await,
            Ok(())
        );
        initial.locked.retain(|locked| locked != LEGACY);
        initial.mutations.push(format!("unlock:{LEGACY}"));
        assert_eq!(*state.lock().unwrap(), initial);
    });
}

#[derive(Debug, Clone)]
struct FakePrompt {
    state: Arc<Mutex<State>>,
    dismissed: Option<bool>,
}

#[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
impl FakePrompt {
    async fn prompt(
        &self,
        _window: &str,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.state.lock().unwrap().mutations.push("prompt".into());
        if let Some(dismissed) = self.dismissed {
            Self::completed(&emitter, dismissed, OwnedValue::from(false)).await?;
        }
        Ok(())
    }

    fn dismiss(&self) {
        self.state.lock().unwrap().mutations.push("dismiss".into());
    }

    #[zbus(signal)]
    async fn completed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        dismissed: bool,
        result: OwnedValue,
    ) -> zbus::Result<()>;
}

#[test]
fn prompts_complete_cancel_or_time_out_and_are_never_shown_noninteractively() {
    pollster::block_on(async {
        for (interactive, dismissed, expected, mutations) in [
            (true, Some(false), Ok(()), vec!["prompt"]),
            (true, Some(true), Err(Error::Cancelled), vec![
                "prompt", "dismiss",
            ]),
            (true, None, Err(Error::TimedOut), vec!["prompt", "dismiss"]),
            (false, None, Err(Error::Locked), vec!["dismiss"]),
        ] {
            let mut initial = initial_state();
            let state = Arc::new(Mutex::new(initial.clone()));
            let (mut client, server) = private_service(state.clone()).await;
            client.interactive = interactive;
            client.prompt_timeout = Duration::from_secs(if dismissed.is_some() { 10 } else { 1 });
            server
                .object_server()
                .at("/prompt/test", FakePrompt {
                    state: state.clone(),
                    dismissed,
                })
                .await
                .unwrap();
            assert_eq!(
                client
                    .complete_prompt(path("/prompt/test"))
                    .await
                    .map(|_| ()),
                expected
            );
            initial.mutations = mutations.into_iter().map(String::from).collect();
            assert_eq!(*state.lock().unwrap(), initial);
        }
    });
}
