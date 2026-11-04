//! Which window to capture (only the focused window). Platform code lists the
//! on-screen windows of the focused app and this rule picks one or refuses.

/// The index of the focused window among `titles`, the on-screen windows of the focused app
/// ordered front to back. `known` is the Accessibility title of the focused window, empty when
/// the OS did not say.
///
/// With a known title the window must be the only one with that title. Without one the front
/// window is the focused one. Anything else is `None`: a wrong window is never captured.
pub fn focused_window(titles: &[&str], known: &str) -> Option<usize> {
    if known.is_empty() {
        return (!titles.is_empty()).then_some(0);
    }
    let mut matches = titles
        .iter()
        .enumerate()
        .filter(|(_, title)| **title == known);
    match (matches.next(), matches.next()) {
        (Some((index, _)), None) => Some(index),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_window_is_the_focused_one() {
        assert_eq!(focused_window(&["Inbox"], "Inbox"), Some(0));
        assert_eq!(focused_window(&["Inbox"], ""), Some(0));
    }

    #[test]
    fn the_known_title_picks_among_several_windows() {
        let titles = ["Draft", "Inbox", "Calendar"];
        assert_eq!(focused_window(&titles, "Inbox"), Some(1));
        assert_eq!(focused_window(&titles, "Calendar"), Some(2));
    }

    #[test]
    fn a_known_title_that_matches_nothing_captures_nothing() {
        assert_eq!(focused_window(&["Draft", "Inbox"], "Banking"), None);
        assert_eq!(focused_window(&["Inbox"], "Banking"), None);
    }

    #[test]
    fn two_windows_with_the_known_title_are_ambiguous() {
        assert_eq!(focused_window(&["Notes", "Inbox", "Notes"], "Notes"), None);
    }

    #[test]
    fn without_a_known_title_the_front_window_is_the_focused_one() {
        assert_eq!(focused_window(&["Draft", "Inbox"], ""), Some(0));
    }

    #[test]
    fn no_window_captures_nothing() {
        assert_eq!(focused_window(&[], ""), None);
        assert_eq!(focused_window(&[], "Inbox"), None);
    }
}
