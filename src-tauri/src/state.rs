//! Shared application state (diff sessions, sidecar handles).

use std::sync::Mutex;

/// Handle for a running diff job (cancelation + progress events).
#[derive(Default)]
pub struct AppState {
    /// Generation counter for diff jobs; used for cancelation.
    pub diff_generation: Mutex<u64>,
}
