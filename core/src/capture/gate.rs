//! Gates (plan.md §4). They run in this order, each before the step that would read more:
//! [`session`] before any OS call, [`app`] before the window is read, [`secure_input`] and [`tab`]
//! before a frame exists. A failed gate means no capture call is made at all.

use super::browser::Browser;
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    Paused,
    SourceOff,
    ExcludedApp,
    /// Safari and other browsers that cannot report a private window, unless the user opted in.
    OpaqueBrowser,
    PrivateWindow,
    /// The browser did not say whether the window is private, so it counts as private.
    TabUnknown,
    SecureInput,
}

impl Blocked {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::SourceOff => "source off",
            Self::ExcludedApp => "excluded app",
            Self::OpaqueBrowser => "opaque browser",
            Self::PrivateWindow => "private window",
            Self::TabUnknown => "tab unknown",
            Self::SecureInput => "secure input",
        }
    }
}

/// Paused, or every live source off.
pub fn session(settings: &Settings, paused: bool) -> Result<(), Blocked> {
    if paused {
        Err(Blocked::Paused)
    } else if !settings.live.any() {
        Err(Blocked::SourceOff)
    } else {
        Ok(())
    }
}

/// Excluded apps (password managers, Health, messengers by default), and browsers that cannot
/// tell a private window apart unless the user opted in.
pub fn app(settings: &Settings, name: &str, browser: Option<Browser>) -> Result<(), Blocked> {
    if settings.is_excluded(name) {
        Err(Blocked::ExcludedApp)
    } else if browser == Some(Browser::Opaque) && !settings.live.opaque_browsers {
        Err(Blocked::OpaqueBrowser)
    } else {
        Ok(())
    }
}

/// A keyboard focused on a password field or anything else that asked for secure input.
pub fn secure_input(on: bool) -> Result<(), Blocked> {
    if on {
        Err(Blocked::SecureInput)
    } else {
        Ok(())
    }
}

/// `private` is `None` when a browser that should know did not answer.
pub fn tab(private: Option<bool>) -> Result<(), Blocked> {
    match private {
        Some(true) => Err(Blocked::PrivateWindow),
        None => Err(Blocked::TabUnknown),
        Some(false) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_gate_names_what_closed_it() {
        let mut settings = Settings::default();
        assert_eq!(session(&settings, true), Err(Blocked::Paused));
        assert_eq!(session(&settings, false), Ok(()));
        settings.live.metadata = false;
        settings.live.ocr = false;
        settings.live.copy = false;
        assert_eq!(session(&settings, false), Err(Blocked::SourceOff));

        let settings = Settings::default();
        assert_eq!(
            app(&settings, "1Password 8", None),
            Err(Blocked::ExcludedApp)
        );
        assert_eq!(
            app(&settings, "Safari", Some(Browser::Opaque)),
            Err(Blocked::OpaqueBrowser)
        );
        assert_eq!(app(&settings, "Arc", Some(Browser::Arc)), Ok(()));

        assert_eq!(tab(Some(true)), Err(Blocked::PrivateWindow));
        assert_eq!(tab(None), Err(Blocked::TabUnknown));
        assert_eq!(tab(Some(false)), Ok(()));
        assert_eq!(secure_input(true), Err(Blocked::SecureInput));
        assert_eq!(secure_input(false), Ok(()));
    }

    #[test]
    fn opting_in_opens_the_opaque_browsers() {
        let mut settings = Settings::default();
        settings.live.opaque_browsers = true;
        assert_eq!(app(&settings, "Safari", Some(Browser::Opaque)), Ok(()));
    }
}
