//! Which bundle ids are browsers, and how much a browser tells us about its private windows.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    /// Chrome, Brave, Edge: `mode of front window` is `incognito` for a private window.
    Chromium,
    /// Arc: `incognito of front window`.
    Arc,
    /// Safari, Firefox and others: no way to ask whether the window is private.
    Opaque,
}

const CHROMIUM: [&str; 3] = [
    "com.google.Chrome",
    "com.brave.Browser",
    "com.microsoft.edgemac",
];
const OPAQUE: [&str; 9] = [
    "com.apple.Safari",
    "org.mozilla.firefox",
    "org.mozilla.nightly",
    "org.torproject.torbrowser",
    "com.kagi.kagimacOS",
    "com.duckduckgo.macos.browser",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
    "org.chromium.Chromium",
];

/// `None` when the bundle id is not a browser. Beta and Canary builds share the prefix.
pub fn classify(bundle_id: &str) -> Option<Browser> {
    if CHROMIUM.iter().any(|id| bundle_id.starts_with(id)) {
        Some(Browser::Chromium)
    } else if bundle_id == "company.thebrowser.Browser" {
        Some(Browser::Arc)
    } else if OPAQUE.iter().any(|id| bundle_id.starts_with(id)) {
        Some(Browser::Opaque)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_are_told_apart_by_what_they_report() {
        assert_eq!(
            classify("com.google.Chrome.canary"),
            Some(Browser::Chromium)
        );
        assert_eq!(classify("company.thebrowser.Browser"), Some(Browser::Arc));
        assert_eq!(classify("com.apple.Safari"), Some(Browser::Opaque));
        assert_eq!(classify("com.apple.mail"), None);
    }
}
