//! OS access for backfill (plan.md §6, §12). `cfg` appears only here: macOS gets the real
//! implementation, other systems get one that reports "needs macOS" so the crate still builds.

#[cfg(target_os = "macos")]
mod eventkit;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(target_os = "macos")]
pub use macos::{
    System, calendar_status, open_settings_pane, pick_folder, reminders_status, request_permission,
    safari_status,
};
#[cfg(not(target_os = "macos"))]
pub use unsupported::{
    System, calendar_status, open_settings_pane, pick_folder, reminders_status, request_permission,
    safari_status,
};
