use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Sextant", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Sextant", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(true)
        .tooltip("Sextant")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Err(err) = open_window(app) {
                    eprintln!("opening the window failed: {err}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        return window.set_focus();
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Sextant")
        .inner_size(880.0, 620.0)
        .min_inner_size(560.0, 420.0)
        .build()?;
    Ok(())
}
