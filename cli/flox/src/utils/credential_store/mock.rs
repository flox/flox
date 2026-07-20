//! The in-memory backend for tests, with optional error injection.

use std::sync::{Arc, Mutex};

use super::{CredentialStore, CredentialStoreError};

/// In-memory credential store for tests, with optional error injection.
#[derive(Debug, Clone, Default)]
pub struct MockStore {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Debug, Default)]
struct MockState {
    token: Option<String>,
    error: Option<String>,
    remove_error: Option<String>,
}

impl MockStore {
    // Test-only constructor; exercised by the orchestration tests in Phase 2/3
    // and this module's tests. `cli/flox` is a binary crate, so `pub` does not
    // exempt it from dead-code analysis (mirrors `set_lock_results` in the SDK).
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject an error returned by the next `get`/`set`/`remove` call.
    #[allow(dead_code)]
    pub fn set_error(&self, message: impl Into<String>) {
        self.inner.lock().unwrap().error = Some(message.into());
    }

    /// Make every `remove` call fail. Unlike [Self::set_error] (one-shot, on the
    /// next call) this persists, so a test can have `get` succeed while `remove`
    /// fails — the shape of a plaintext file that is readable but cannot be
    /// rewritten.
    #[allow(dead_code)]
    pub fn set_remove_error(&self, message: impl Into<String>) {
        self.inner.lock().unwrap().remove_error = Some(message.into());
    }

    fn take_error(&self) -> Option<CredentialStoreError> {
        self.inner
            .lock()
            .unwrap()
            .error
            .take()
            .map(CredentialStoreError::Mock)
    }
}

impl CredentialStore for MockStore {
    fn get(&self) -> Result<Option<String>, CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        Ok(self.inner.lock().unwrap().token.clone())
    }

    fn set(&self, token: &str) -> Result<(), CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        self.inner.lock().unwrap().token = Some(token.to_string());
        Ok(())
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        let mut state = self.inner.lock().unwrap();
        if let Some(message) = &state.remove_error {
            return Err(CredentialStoreError::Mock(message.clone()));
        }
        state.token = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::utils::credential_store::test_helpers::TOKEN;

    #[test]
    fn mock_set_get_remove_round_trip() {
        let store = MockStore::new();
        assert_eq!(store.get().unwrap(), None);

        store.set(TOKEN).unwrap();
        assert_eq!(store.get().unwrap(), Some(TOKEN.to_string()));

        store.remove().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn mock_injects_error() {
        let store = MockStore::new();
        store.set_error("boom");

        let result = store.get();
        assert_eq!(
            result.unwrap_err().to_string(),
            CredentialStoreError::Mock("boom".to_string()).to_string()
        );

        // The injected error is consumed; subsequent calls succeed.
        assert_eq!(store.get().unwrap(), None);
    }
}
