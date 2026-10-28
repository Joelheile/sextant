mod home;
mod tray;

use anyhow::Context;
use tauri::RunEvent;

fn main() -> anyhow::Result<()> {
    home::data_dir()?;
    tauri::Builder::default()
        .setup(|app| {
            tray::build(app.handle())?;
            tray::open_window(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("building the Tauri app")?
        .run(|_, event| {
            // Closing the window destroys it; the tray keeps running until Quit.
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
    Ok(())
}
