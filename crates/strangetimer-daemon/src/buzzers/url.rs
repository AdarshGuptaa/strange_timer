use strangetimer_core::model::SessionEnv;

/// Open a URL in the OS default browser, launched under `env` — the
/// latest-known interactive session (see `platform::open_target`). Returns
/// a short error string when the opener failed (surfaced as a
/// `BuzzerEvent::outcome`).
pub fn fire_url(url: &str, env: &SessionEnv) -> Option<String> {
    crate::platform::open_target(url, env)
}
