//! Backend half of the in-app updater. Checking, downloading and verifying
//! are done by tauri-plugin-updater from the frontend; this only reads the
//! user's setting and closes the shell before the installer takes over.

use crate::{config, pty::PtyState};
use tauri::State;

/// Whether to check for updates on startup (`checkForUpdates` in config.yml, default on).
/// "Check for updates" in the sidebar works either way.
#[tauri::command]
pub async fn updates_auto_check() -> Result<bool, String> {
    Ok(config::load().0.check_for_updates.unwrap_or(true))
}

/// Called right before installing. Refuses while a recipe is running (or stopped
/// on a conflict), so an update never cuts off a push or leaves a merge half done.
/// Otherwise ends the shell, because the Windows installer exits the app without
/// running the normal shutdown.
#[tauri::command]
pub async fn updates_prepare_install(state: State<'_, PtyState>) -> Result<(), String> {
    {
        let s = state.shared.lock().unwrap();
        if s.runner.run.is_some() || s.runner.suspended.is_some() {
            return Err("A recipe is running. Finish or stop it, then update.".into());
        }
    }
    state.shutdown();
    Ok(())
}
