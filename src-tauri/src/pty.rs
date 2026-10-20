//! The single interactive shell session behind the terminal view.
//!
//! Output streams to the frontend over a Tauri `Channel` as UTF-8 text (split
//! multi-byte sequences are carried over between reads). Shell-integration
//! markers are removed from the stream and drive [`crate::runner`]. When the
//! shell exits we emit `pty://exit` after the last output chunk.

use crate::osc::{MarkerParser, Segment};
use crate::runner::Sink;
use crate::{config, runner, shell};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

struct Session {
    generation: u64,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

#[derive(Debug, Default, Clone)]
pub struct ShellStatus {
    /// The prompt hook is installed (bash/zsh with default args).
    pub integration: bool,
    /// At least one prompt marker has arrived.
    pub ready: bool,
    /// At a prompt: the last marker came after the last Enter we saw.
    pub idle: bool,
    pub cwd: Option<PathBuf>,
}

/// Everything the reader thread, the commands and the runner share.
/// One lock keeps the ordering simple.
#[derive(Default)]
pub struct Shared {
    session: Option<Session>,
    pub shell: ShellStatus,
    pub runner: runner::RunnerState,
}

impl Shared {
    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        SessionWriter(&mut self.session).send(data)
    }

    pub fn generation(&self) -> Option<u64> {
        self.session.as_ref().map(|s| s.generation)
    }

    /// Borrows the parts the runner needs at the same time.
    pub fn split(&mut self) -> (&mut ShellStatus, &mut runner::RunnerState, SessionWriter<'_>) {
        (&mut self.shell, &mut self.runner, SessionWriter(&mut self.session))
    }
}

pub struct SessionWriter<'a>(&'a mut Option<Session>);

impl runner::Sink for SessionWriter<'_> {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        let session = self.0.as_mut().ok_or("No shell is running")?;
        session.writer.write_all(data).map_err(|e| e.to_string())?;
        session.writer.flush().map_err(|e| e.to_string())
    }
}

#[derive(Default)]
pub struct PtyState {
    pub shared: Arc<Mutex<Shared>>,
    generation: AtomicU64,
}

impl PtyState {
    pub fn shutdown(&self) {
        let mut s = self.shared.lock().unwrap();
        s.session.take();
        s.runner = runner::RunnerState::default();
        s.shell = ShellStatus::default();
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        self.shared.lock().unwrap().shell.cwd.clone()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnInfo {
    shell: String,
    args: Vec<String>,
    cwd: String,
    integration: bool,
    windows_build: Option<u32>,
}

#[derive(Clone, Serialize)]
struct PtyExit {
    code: Option<u32>,
}

fn windows_build() -> Option<u32> {
    if !cfg!(windows) {
        return None;
    }
    match os_info::get().version() {
        os_info::Version::Semantic(_, _, build) => u32::try_from(*build).ok(),
        _ => None,
    }
}

/// Where the shell starts: a folder given on the command line (Explorer's
/// "Open Branchdial here", or `branchdial <dir>`), else `startDir` from the
/// config, else the home directory.
fn start_dir(cfg: &config::Config) -> PathBuf {
    launch_dir()
        .cloned()
        .or_else(|| cfg.start_dir.as_deref().map(config::expand_home).filter(|p| p.is_dir()))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The folder from the command line, read once (restarting the shell reuses it).
fn launch_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| dir_from_args(std::env::args_os().skip(1))).as_ref()
}

/// The first argument that names an existing folder (a file means its folder).
/// Options (`-x`) are skipped, e.g. the `-psn_…` macOS may pass.
fn dir_from_args(args: impl Iterator<Item = OsString>) -> Option<PathBuf> {
    for arg in args {
        let arg = arg.to_string_lossy();
        if arg.starts_with('-') {
            continue;
        }
        // Explorer passes "%V" quoted, and for a drive root `"C:\"` the `\"`
        // escapes the closing quote, so it arrives as `C:"`.
        let mut s = arg.trim().trim_end_matches('"').to_string();
        if s.len() == 2 && s.ends_with(':') {
            s.push('\\');
        }
        let path = config::expand_home(&s);
        if path.is_dir() {
            return Some(path);
        }
        if path.is_file() {
            if let Some(parent) = path.parent() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

/// 128 bits from the OS's secure random source: program output can't guess it,
/// so it can't fake a "command finished" marker.
pub(crate) fn new_nonce() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts (or restarts) the shell. Any existing session is killed first.
#[tauri::command]
pub async fn pty_spawn(
    app: AppHandle,
    state: State<'_, PtyState>,
    cols: u16,
    rows: u16,
    on_data: Channel<String>,
) -> Result<SpawnInfo, String> {
    state.shutdown();

    let (cfg, warning) = config::load();
    if let Some(w) = warning {
        let _ = on_data.send(format!("\x1b[33m{w}\x1b[0m\r\n"));
    }
    let spec = shell::resolve(&cfg)?;
    let cwd = start_dir(&cfg);
    let nonce = new_nonce();
    let launch = shell::prepare_launch(&spec, &nonce);

    let pair = native_pty_system()
        .openpty(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Could not open a pseudo-terminal: {e}"))?;

    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&launch.args);
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "branchdial");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Could not start {}: {e}", spec.program.display()))?;
    // The slave end belongs to the child now; holding it would keep the pty open after exit.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let killer = child.clone_killer();

    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut s = state.shared.lock().unwrap();
        s.session = Some(Session { generation, master: pair.master, writer, killer });
        s.shell = ShellStatus { integration: launch.integration, ready: false, idle: false, cwd: Some(cwd.clone()) };
        s.runner = runner::RunnerState::default();
    }

    let shared = state.shared.clone();
    let reader_app = app.clone();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let mut pending = Vec::new();
        let mut parser = MarkerParser::new(nonce);
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for seg in parser.feed(&buf[..n]) {
                        match seg {
                            Segment::Output(bytes) => {
                                let text = decode_utf8_stream(&mut pending, &bytes);
                                if !text.is_empty() && on_data.send(text).is_err() {
                                    return;
                                }
                            }
                            Segment::Prompt(p) => runner::on_prompt(&reader_app, &shared, generation, p),
                        }
                    }
                }
            }
        }
    });

    let shared = state.shared.clone();
    std::thread::spawn(move || {
        let status = child.wait().ok();
        // Let ConPTY flush its last frame, then close the master so the reader sees EOF.
        std::thread::sleep(Duration::from_millis(100));
        let (current, interrupted_run) = {
            let mut s = shared.lock().unwrap();
            if s.session.as_ref().map(|x| x.generation) == Some(generation) {
                s.session.take();
                s.shell = ShellStatus::default();
                (true, s.runner.run.take().is_some())
            } else {
                (false, false)
            }
        };
        let _ = reader_thread.join();
        // A replaced session must not tell the UI that "the" shell exited.
        if current {
            if interrupted_run {
                runner::emit_shell_exited(&app);
            }
            let _ = app.emit("pty://exit", PtyExit { code: status.map(|s| s.exit_code()) });
        }
    });

    Ok(SpawnInfo {
        shell: spec.program.display().to_string(),
        args: launch.args,
        cwd: cwd.display().to_string(),
        integration: launch.integration,
        windows_build: windows_build(),
    })
}

#[tauri::command]
pub async fn pty_write(state: State<'_, PtyState>, data: String) -> Result<(), String> {
    let mut s = state.shared.lock().unwrap();
    if data.contains(['\r', '\n']) {
        s.shell.idle = false;
    }
    if data.contains('\x03') {
        if let Some(run) = s.runner.run.as_mut() {
            run.cancel_requested = true;
        }
    }
    s.write(data.as_bytes())
}

#[tauri::command]
pub async fn pty_resize(state: State<'_, PtyState>, cols: u16, rows: u16) -> Result<(), String> {
    let s = state.shared.lock().unwrap();
    let Some(session) = s.session.as_ref() else { return Ok(()) };
    session
        .master
        .resize(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

/// Appends `chunk` to `pending` and returns everything that decodes cleanly.
/// An incomplete multi-byte sequence at the end stays in `pending` for the next
/// read; truly invalid bytes become U+FFFD.
fn decode_utf8_stream(pending: &mut Vec<u8>, chunk: &[u8]) -> String {
    pending.extend_from_slice(chunk);
    let mut out = String::with_capacity(pending.len());
    let mut start = 0;
    while start < pending.len() {
        match std::str::from_utf8(&pending[start..]) {
            Ok(s) => {
                out.push_str(s);
                start = pending.len();
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // from_utf8 just told us this prefix is valid.
                out.push_str(std::str::from_utf8(&pending[start..start + valid]).unwrap());
                match e.error_len() {
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        start += valid + bad;
                    }
                    None => {
                        start += valid;
                        break;
                    }
                }
            }
        }
    }
    pending.drain(..start);
    out
}

#[cfg(test)]
mod tests {
    use super::{decode_utf8_stream, dir_from_args};
    use std::ffi::OsString;

    fn args(list: &[&str]) -> std::vec::IntoIter<OsString> {
        list.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn folder_from_command_line() {
        let tmp = std::env::temp_dir();
        let t = tmp.to_string_lossy().to_string();
        assert_eq!(dir_from_args(args(&[&t])), Some(tmp.clone()));
        // Flags are skipped; a quote left over from Explorer is tolerated.
        assert_eq!(dir_from_args(args(&["-psn_0_123", &format!("{t}\"")])).as_deref(), Some(std::path::Path::new(&t)));
        // A file opens its folder.
        let file = tmp.join(format!("branchdial-arg-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert_eq!(dir_from_args(args(&[&file.to_string_lossy()])), Some(tmp.clone()));
        let _ = std::fs::remove_file(&file);
        // Nothing usable: fall back to the config / home.
        assert_eq!(dir_from_args(args(&[])), None);
        assert_eq!(dir_from_args(args(&["/definitely/not/here"])), None);
    }

    /// Explorer's `"%V"` for a drive root arrives as `C:"`.
    #[cfg(windows)]
    #[test]
    fn drive_root_from_explorer() {
        assert_eq!(dir_from_args(args(&["C:\""])), Some(std::path::PathBuf::from("C:\\")));
    }

    #[test]
    fn passes_ascii_through() {
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, b"hello"), "hello");
        assert!(p.is_empty());
    }

    #[test]
    fn carries_split_multibyte_sequence() {
        let bytes = "a→b".as_bytes(); // → is 3 bytes
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, &bytes[..2]), "a");
        assert_eq!(p.len(), 1);
        assert_eq!(decode_utf8_stream(&mut p, &bytes[2..3]), "");
        assert_eq!(decode_utf8_stream(&mut p, &bytes[3..]), "→b");
        assert!(p.is_empty());
    }

    #[test]
    fn replaces_invalid_bytes() {
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, b"a\xffb"), "a\u{FFFD}b");
        assert!(p.is_empty());
    }
}
