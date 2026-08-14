/// Open a URL in the OS default browser.
pub fn fire_url(url: &str) {
    if let Err(e) = open::that(url) {
        warn!("failed to open URL {url:?}: {e}");
    }
}
