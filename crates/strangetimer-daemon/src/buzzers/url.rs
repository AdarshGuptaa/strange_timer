/// Open a URL in the OS default browser.
pub fn fire_url(url: &str) {
    crate::platform::open_target(url);
}
