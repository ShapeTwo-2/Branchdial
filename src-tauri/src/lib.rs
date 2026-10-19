mod config;
mod git;
mod osc;
mod pty;
mod recipes;
mod runner;
mod safety;
mod shell;
mod status;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(pty::PtyState::default())
        .manage(status::StatusWatcher::default())
        .invoke_handler(tauri::generate_handler![
            pty::pty_spawn,
            pty::pty_write,
            pty::pty_resize,
            recipes::recipes_list,
            recipes::recipe_plan,
            recipes::recipe_run,
            recipes::git_branches,
            runner::recipe_cancel,
            runner::conflict_files,
            runner::conflict_continue,
            runner::conflict_abort,
            recipes::repo_trust,
            git::git_info,
            status::status_refresh,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Branchdial")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<pty::PtyState>().shutdown();
            }
        });
}
