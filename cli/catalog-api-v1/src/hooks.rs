use std::sync::Arc;

use progenitor_client::{ClientHooks, Error, OperationInfo};

/// Per-instance request hooks embedded in the generated `Client` via
/// `with_inner_type`.
///
/// This replaces the former global `Mutex<Option<Hook>>` in
/// `pre_request_hook.rs`, giving each `Client` instance its own hook without
/// shared mutable state.
pub struct RequestHooks {
    pub pre_request: Arc<dyn Fn(&mut reqwest::Request) + Send + Sync>,
}

impl Clone for RequestHooks {
    fn clone(&self) -> Self {
        Self {
            pre_request: Arc::clone(&self.pre_request),
        }
    }
}

impl std::fmt::Debug for RequestHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestHooks")
            .field("pre_request", &"<closure>")
            .finish()
    }
}

impl Default for RequestHooks {
    fn default() -> Self {
        Self {
            pre_request: Arc::new(|_| {}),
        }
    }
}

impl ClientHooks<RequestHooks> for crate::Client {
    async fn pre<E>(
        &self,
        request: &mut reqwest::Request,
        _info: &OperationInfo,
    ) -> Result<(), Error<E>> {
        (self.inner.pre_request)(request);
        Ok(())
    }
}
