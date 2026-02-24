//! A global pre-request hook for injecting per-request logic (e.g. auth headers)
//! into every outgoing API request.
//!
//! This is called from the generated client's `pre_hook_async` before each request.

use std::sync::Mutex;

type Hook = Box<dyn Fn(&mut reqwest::Request) + Send + Sync>;

static PRE_REQUEST_HOOK: Mutex<Option<Hook>> = Mutex::new(None);

/// Register a function to be called before every outgoing API request.
///
/// The hook receives a mutable reference to the [`reqwest::Request`] and may
/// modify headers, query parameters, etc.
///
/// Replaces any previously registered hook.
pub fn set_pre_request_hook(hook: impl Fn(&mut reqwest::Request) + Send + Sync + 'static) {
    *PRE_REQUEST_HOOK.lock().expect("pre-request hook lock poisoned") = Some(Box::new(hook));
}

/// Run the registered pre-request hook, if any.
///
/// Called from the generated client code via `pre_hook_async`.
pub fn run_pre_request_hook(request: &mut reqwest::Request) {
    if let Some(hook) = PRE_REQUEST_HOOK
        .lock()
        .expect("pre-request hook lock poisoned")
        .as_ref()
    {
        hook(request);
    }
}
