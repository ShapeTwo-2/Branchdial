//! Runs a recipe's steps one at a time by typing them into the shell.
//!
//! A step is typed only when the shell is at a prompt. The next prompt marker
//! carries that step's exit code: 0 moves on to the next step; anything else
//! stops the recipe, after which only `always: true` steps still run (and not
//! even those while git is mid-merge/rebase).
//!
//! A recipe stopped by a merge/rebase conflict is kept as "suspended": the
//! conflict UI can continue it (finish the operation, then the remaining
//! steps) or abort it and return to the branch the recipe started on.

use crate::git::{self, Operation};
use crate::osc::Prompt;
use crate::pty::{PtyState, Shared, ShellStatus};
use crate::recipes::plan::PlannedStep;
use crate::recipes::template::sh_quote;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

/// Where typed commands go: the PTY in the app, a pipe in tests.
pub trait Sink {
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    Pending,
    Running,
    Ok,
    Failed,
    Skipped,
}

#[derive(Debug)]
pub struct Run {
    name: String,
    start_branch: Option<String>,
    steps: Vec<PlannedStep>,
    states: Vec<StepState>,
    current: usize,
    failure: Option<(usize, i32)>,
    /// Set when the user presses Ctrl+C (or clicks Stop) during a run.
    pub cancel_requested: bool,
}

/// A recipe stopped by a conflict, waiting for Resolve → Continue or Abort.
#[derive(Debug, Clone)]
pub struct Suspended {
    pub name: String,
    pub remaining: Vec<PlannedStep>,
    pub start_branch: Option<String>,
}

#[derive(Debug, Default)]
pub struct RunnerState {
    pub run: Option<Run>,
    pub suspended: Option<Suspended>,
}

/// Name and starting point of a run.
pub struct RunMeta {
    pub name: String,
    /// The branch checked out when the recipe started, for "go back".
    pub start_branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Ok,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptEvent {
    pub exit_code: i32,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStarted {
    pub name: String,
    pub steps: Vec<PlannedStep>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    pub index: usize,
    pub state: StepState,
    pub exit_code: Option<i32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDone {
    pub status: RunStatus,
    pub failed_step: Option<usize>,
    pub exit_code: Option<i32>,
    /// A merge/rebase/... left waiting for the user.
    pub operation: Option<Operation>,
    /// Steps that will run after Resolve → Continue.
    pub remaining: usize,
    /// Where Abort & go back returns to.
    pub start_branch: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug)]
pub enum Event {
    Prompt(PromptEvent),
    Started(RunStarted),
    Step(StepEvent),
    Done(RunDone),
}

pub fn emit(app: &AppHandle, events: Vec<Event>) {
    for e in events {
        let _ = match e {
            Event::Prompt(p) => app.emit("shell://prompt", p),
            Event::Started(s) => app.emit("run://started", s),
            Event::Step(s) => app.emit("run://step", s),
            Event::Done(d) => app.emit("run://done", d),
        };
    }
}

pub fn emit_shell_exited(app: &AppHandle) {
    emit(
        app,
        vec![Event::Done(RunDone {
            status: RunStatus::Failed,
            failed_step: None,
            exit_code: None,
            operation: None,
            remaining: 0,
            start_branch: None,
            message: Some("The shell exited while the recipe was running.".into()),
        })],
    );
}

/// Ctrl+E Ctrl+U: clear anything half-typed on the prompt line before typing a step.
const CLEAR_LINE: &str = "\x05\x15";

fn type_step(run: &mut Run, shell: &mut ShellStatus, sink: &mut dyn Sink, index: usize) -> Result<Event, String> {
    sink.send(format!("{CLEAR_LINE}{}\r", run.steps[index].command).as_bytes())?;
    shell.idle = false;
    run.current = index;
    run.states[index] = StepState::Running;
    Ok(Event::Step(StepEvent { index, state: StepState::Running, exit_code: None, note: None }))
}

/// Starts a run. Refuses unless the shell is integrated, at a prompt and not already running one.
pub fn start(
    shell: &mut ShellStatus,
    state: &mut RunnerState,
    sink: &mut dyn Sink,
    meta: RunMeta,
    steps: Vec<PlannedStep>,
) -> Result<Vec<Event>, String> {
    if !shell.integration {
        return Err("Recipes need bash or zsh with Branchdial's shell integration (see ~/.branchdial/config.yml).".into());
    }
    if state.run.is_some() {
        return Err("A recipe is already running.".into());
    }
    if !shell.ready || !shell.idle {
        return Err("The shell is busy. Wait for the prompt (or press Ctrl+C) and try again.".into());
    }
    if steps.is_empty() {
        return Err("Nothing to run.".into());
    }
    let mut run = Run {
        name: meta.name,
        start_branch: meta.start_branch,
        states: vec![StepState::Pending; steps.len()],
        steps,
        current: 0,
        failure: None,
        cancel_requested: false,
    };
    let mut events = vec![Event::Started(RunStarted { name: run.name.clone(), steps: run.steps.clone() })];
    events.push(type_step(&mut run, shell, sink, 0)?);
    state.run = Some(run);
    state.suspended = None;
    Ok(events)
}

/// Handles a prompt marker: the shell is idle again, and if a step was running it just finished.
pub fn handle_prompt(shell: &mut ShellStatus, state: &mut RunnerState, sink: &mut dyn Sink, prompt: Prompt) -> Vec<Event> {
    shell.ready = true;
    shell.idle = true;
    if !prompt.cwd.is_empty() {
        shell.cwd = Some(PathBuf::from(&prompt.cwd));
    }
    let mut events = vec![Event::Prompt(PromptEvent { exit_code: prompt.exit_code, cwd: prompt.cwd })];
    let Some(run) = state.run.as_mut() else { return events };

    let idx = run.current;
    let code = prompt.exit_code;
    let ok = code == 0 && !run.cancel_requested;
    run.states[idx] = if ok { StepState::Ok } else { StepState::Failed };
    events.push(Event::Step(StepEvent { index: idx, state: run.states[idx], exit_code: Some(code), note: None }));
    if !ok && run.failure.is_none() {
        run.failure = Some((idx, code));
    }

    let cwd = shell.cwd.clone();
    let mut op_checked = false;
    let mut op: Option<Operation> = None;
    for next in idx + 1..run.steps.len() {
        let skip_note = if run.cancel_requested {
            Some("cancelled")
        } else if run.failure.is_none() {
            None
        } else if !run.steps[next].always {
            Some("an earlier step failed")
        } else {
            if !op_checked {
                op = cwd.as_deref().and_then(git::operation_in_progress);
                op_checked = true;
            }
            op.map(|_| "git is in the middle of a merge/rebase")
        };
        if let Some(note) = skip_note {
            run.states[next] = StepState::Skipped;
            events.push(Event::Step(StepEvent {
                index: next,
                state: StepState::Skipped,
                exit_code: None,
                note: Some(note.into()),
            }));
            continue;
        }
        match type_step(run, shell, sink, next) {
            Ok(e) => {
                events.push(e);
                return events;
            }
            Err(e) => {
                run.failure.get_or_insert((next, -1));
                run.states[next] = StepState::Failed;
                events.push(Event::Step(StepEvent { index: next, state: StepState::Failed, exit_code: None, note: Some(e) }));
                break;
            }
        }
    }

    // Finished.
    let status = if run.cancel_requested {
        RunStatus::Cancelled
    } else if run.failure.is_some() {
        RunStatus::Failed
    } else {
        RunStatus::Ok
    };
    let operation = if status == RunStatus::Ok {
        None
    } else if op_checked {
        op
    } else {
        cwd.as_deref().and_then(git::operation_in_progress)
    };
    let run = state.run.take().unwrap();
    let mut remaining = 0;
    state.suspended = match (status, operation, run.failure) {
        (RunStatus::Failed, Some(_), Some((failed, _))) => {
            let rest = run.steps[failed + 1..].to_vec();
            remaining = rest.len();
            Some(Suspended { name: run.name.clone(), remaining: rest, start_branch: run.start_branch.clone() })
        }
        _ => None,
    };
    events.push(Event::Done(RunDone {
        status,
        failed_step: run.failure.map(|f| f.0),
        exit_code: run.failure.map(|f| f.1),
        operation,
        remaining,
        start_branch: run.start_branch,
        message: None,
    }));
    events
}

/// Called by the PTY reader thread for each prompt marker.
pub fn on_prompt(app: &AppHandle, shared: &Arc<Mutex<Shared>>, generation: u64, prompt: Prompt) {
    let (events, cwd) = {
        let mut s = shared.lock().unwrap();
        if s.generation() != Some(generation) {
            return;
        }
        let (shell, runner, mut sink) = s.split();
        let events = handle_prompt(shell, runner, &mut sink, prompt);
        (events, shell.cwd.clone())
    };
    emit(app, events);
    // Any command may have changed the repo; refresh the status bar.
    if let Some(cwd) = cwd {
        app.state::<crate::status::StatusWatcher>().request(app, cwd);
    }
}

pub fn start_in_app(app: &AppHandle, state: &State<'_, PtyState>, meta: RunMeta, steps: Vec<PlannedStep>) -> Result<(), String> {
    let events = {
        let mut s = state.shared.lock().unwrap();
        let (shell, runner, mut sink) = s.split();
        start(shell, runner, &mut sink, meta, steps)?
    };
    emit(app, events);
    Ok(())
}

/// Stops a run: Ctrl+C to the shell; the next prompt marker finishes it as cancelled.
#[tauri::command]
pub async fn recipe_cancel(state: State<'_, PtyState>) -> Result<(), String> {
    let mut s = state.shared.lock().unwrap();
    let Some(run) = s.runner.run.as_mut() else { return Ok(()) };
    run.cancel_requested = true;
    s.write(b"\x03")
}

fn repo_root(state: &State<'_, PtyState>) -> Result<PathBuf, String> {
    state
        .cwd()
        .as_deref()
        .and_then(git::repo_root)
        .ok_or_else(|| "The terminal is not inside a git repository.".into())
}

/// The command that finishes an operation once its conflicts are resolved.
/// GIT_EDITOR=true accepts the prepared message instead of opening an editor.
pub fn continue_command(op: Operation) -> &'static str {
    match op {
        Operation::Merge => "git commit --no-edit",
        Operation::Rebase => "GIT_EDITOR=true git rebase --continue",
        Operation::CherryPick => "GIT_EDITOR=true git cherry-pick --continue",
        Operation::Revert => "GIT_EDITOR=true git revert --continue",
    }
}

/// Steps for Abort & go back.
pub fn abort_steps(op: Operation, start_branch: Option<&str>) -> Vec<PlannedStep> {
    let step = |c: String| PlannedStep { command: c, always: false };
    let mut steps = vec![step(format!("git {} --abort", op.as_str()))];
    match start_branch {
        Some(b) => steps.push(step(format!("git checkout {}", sh_quote(b)))),
        // rebase --abort already returns to the original branch.
        None if op == Operation::Merge => steps.push(step("git checkout -".into())),
        None => {}
    }
    steps
}

#[tauri::command]
pub async fn conflict_files(state: State<'_, PtyState>) -> Result<Vec<String>, String> {
    Ok(git::conflicted_files(&repo_root(&state)?))
}

/// Finishes the merge/rebase (conflicts must be resolved and staged), then runs
/// the rest of the suspended recipe.
#[tauri::command]
pub async fn conflict_continue(app: AppHandle, state: State<'_, PtyState>) -> Result<(), String> {
    let root = repo_root(&state)?;
    let conflicted = git::conflicted_files(&root);
    if !conflicted.is_empty() {
        return Err(format!(
            "Fix the conflicts and `git add` these files first: {}",
            conflicted.join(", ")
        ));
    }
    let suspended = state.shared.lock().unwrap().runner.suspended.clone();
    let mut steps = vec![];
    if let Some(op) = git::operation_in_progress(&root) {
        steps.push(PlannedStep { command: continue_command(op).into(), always: false });
    }
    let (name, start_branch) = match suspended {
        Some(s) => {
            steps.extend(s.remaining);
            (format!("{} (continued)", s.name), s.start_branch)
        }
        None => ("Continue".to_string(), None),
    };
    if steps.is_empty() {
        return Err("Nothing to continue.".into());
    }
    start_in_app(&app, &state, RunMeta { name, start_branch }, steps)
}

/// Aborts the operation in progress and returns to the branch the recipe started on.
#[tauri::command]
pub async fn conflict_abort(app: AppHandle, state: State<'_, PtyState>) -> Result<(), String> {
    let root = repo_root(&state)?;
    let op = git::operation_in_progress(&root).ok_or("No merge, rebase, cherry-pick or revert is in progress.")?;
    let start_branch = state.shared.lock().unwrap().runner.suspended.as_ref().and_then(|s| s.start_branch.clone());
    let steps = abort_steps(op, start_branch.as_deref());
    start_in_app(&app, &state, RunMeta { name: "Abort & go back".into(), start_branch: None }, steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::TempRepo;
    use crate::osc::{MarkerParser, Segment};
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    struct VecSink(Vec<String>);
    impl Sink for VecSink {
        fn send(&mut self, data: &[u8]) -> Result<(), String> {
            self.0.push(String::from_utf8_lossy(data).into_owned());
            Ok(())
        }
    }

    fn step(cmd: &str, always: bool) -> PlannedStep {
        PlannedStep { command: cmd.into(), always }
    }

    fn meta(name: &str) -> RunMeta {
        RunMeta { name: name.into(), start_branch: Some("feat".into()) }
    }

    fn ready_shell() -> ShellStatus {
        ShellStatus { integration: true, ready: true, idle: true, cwd: None }
    }

    fn prompt(code: i32) -> Prompt {
        Prompt { exit_code: code, cwd: String::new() }
    }

    fn done(events: &[Event]) -> Option<&RunDone> {
        events.iter().find_map(|e| if let Event::Done(d) = e { Some(d) } else { None })
    }

    #[test]
    fn runs_steps_in_order_until_done() {
        let (mut shell, mut st, mut sink) = (ready_shell(), RunnerState::default(), VecSink(vec![]));
        start(&mut shell, &mut st, &mut sink, meta("r"), vec![step("a", false), step("b", false)]).unwrap();
        assert_eq!(sink.0, vec!["\x05\x15a\r"]);
        assert!(!shell.idle);
        // A second start while running is refused.
        assert!(start(&mut shell, &mut st, &mut sink, meta("r"), vec![step("x", false)]).is_err());
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, prompt(0));
        assert!(done(&ev).is_none());
        assert_eq!(sink.0[1], "\x05\x15b\r");
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, prompt(0));
        assert_eq!(done(&ev).unwrap().status, RunStatus::Ok);
        assert!(st.run.is_none() && st.suspended.is_none());
    }

    #[test]
    fn stops_at_first_failure_but_runs_always_steps() {
        let (mut shell, mut st, mut sink) = (ready_shell(), RunnerState::default(), VecSink(vec![]));
        let steps = vec![step("a", false), step("b", false), step("c", true)];
        start(&mut shell, &mut st, &mut sink, meta("r"), steps).unwrap();
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, prompt(1));
        assert!(ev.iter().any(|e| matches!(e, Event::Step(s) if s.index == 1 && s.state == StepState::Skipped)));
        assert_eq!(sink.0.last().unwrap(), "\x05\x15c\r");
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, prompt(0));
        let d = done(&ev).unwrap();
        assert_eq!((d.status, d.failed_step, d.exit_code), (RunStatus::Failed, Some(0), Some(1)));
        assert!(st.suspended.is_none(), "no conflict, nothing to continue");
    }

    #[test]
    fn cancel_skips_everything_else() {
        let (mut shell, mut st, mut sink) = (ready_shell(), RunnerState::default(), VecSink(vec![]));
        start(&mut shell, &mut st, &mut sink, meta("r"), vec![step("a", false), step("b", true)]).unwrap();
        st.run.as_mut().unwrap().cancel_requested = true;
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, prompt(130));
        assert_eq!(done(&ev).unwrap().status, RunStatus::Cancelled);
        assert_eq!(sink.0.len(), 1);
    }

    #[test]
    fn refuses_when_not_ready() {
        let mut sink = VecSink(vec![]);
        let mut busy = ShellStatus { idle: false, ..ready_shell() };
        assert!(start(&mut busy, &mut RunnerState::default(), &mut sink, meta("r"), vec![step("a", false)]).is_err());
        let mut plain = ShellStatus { integration: false, ..ready_shell() };
        assert!(start(&mut plain, &mut RunnerState::default(), &mut sink, meta("r"), vec![step("a", false)]).is_err());
    }

    #[test]
    fn conflict_suspends_remaining_steps() {
        let r = TempRepo::new("suspend");
        r.git(&["checkout", "-q", "-b", "a"]);
        std::fs::write(r.work.join("README.md"), "a\n").unwrap();
        r.git(&["commit", "-q", "-am", "a"]);
        r.git(&["checkout", "-q", "main"]);
        std::fs::write(r.work.join("README.md"), "main\n").unwrap();
        r.git(&["commit", "-q", "-am", "main"]);
        let _ = std::process::Command::new("git").args(["merge", "a"]).current_dir(&r.work).output();

        let mut shell = ShellStatus { cwd: Some(r.work.clone()), ..ready_shell() };
        let (mut st, mut sink) = (RunnerState::default(), VecSink(vec![]));
        let steps = vec![step("git merge a", false), step("git push", false), step("git checkout feat", true)];
        start(&mut shell, &mut st, &mut sink, meta("Merge into…"), steps).unwrap();
        let ev = handle_prompt(&mut shell, &mut st, &mut sink, Prompt { exit_code: 1, cwd: r.work.display().to_string() });
        let d = done(&ev).unwrap();
        assert_eq!(d.operation, Some(Operation::Merge));
        assert_eq!(d.remaining, 2);
        let s = st.suspended.as_ref().unwrap();
        assert_eq!(s.remaining[0].command, "git push");
        assert_eq!(s.start_branch.as_deref(), Some("feat"));
    }

    #[test]
    fn abort_and_continue_commands() {
        let cmds = |s: Vec<PlannedStep>| s.into_iter().map(|p| p.command).collect::<Vec<_>>();
        assert_eq!(cmds(abort_steps(Operation::Merge, Some("feat/x"))), ["git merge --abort", "git checkout 'feat/x'"]);
        assert_eq!(cmds(abort_steps(Operation::Merge, None)), ["git merge --abort", "git checkout -"]);
        assert_eq!(cmds(abort_steps(Operation::Rebase, None)), ["git rebase --abort"]);
        assert_eq!(continue_command(Operation::Rebase), "GIT_EDITOR=true git rebase --continue");
    }

    struct PipeSink(Box<dyn Write + Send>);
    impl Sink for PipeSink {
        fn send(&mut self, data: &[u8]) -> Result<(), String> {
            self.0.write_all(data).map_err(|e| e.to_string())?;
            self.0.flush().map_err(|e| e.to_string())
        }
    }

    /// The whole loop against a real shell with the real integration script:
    /// a bcp-style recipe in a throwaway repo, pushing to a local bare "origin".
    #[test]
    fn end_to_end_with_real_shell() {
        let repo = TempRepo::new("e2e");
        std::fs::write(repo.work.join("new.txt"), "x\n").unwrap();

        let spec = crate::shell::resolve(&crate::config::Config::default()).unwrap();
        if spec.kind != crate::shell::Kind::Bash && spec.kind != crate::shell::Kind::Zsh {
            return; // CI box without bash/zsh as default shell
        }
        let nonce = "testnonce";
        let launch = crate::shell::prepare_launch(&spec, nonce);
        assert!(launch.integration);

        let pair = native_pty_system()
            .openpty(PtySize { rows: 30, cols: 200, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&launch.args);
        cmd.cwd(&repo.work);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in &launch.env {
            cmd.env(k, v);
        }
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(PipeSink(pair.master.take_writer().unwrap())));

        let (tx, rx) = mpsc::channel::<Prompt>();
        let dsr_writer = writer.clone();
        std::thread::spawn(move || {
            let mut parser = MarkerParser::new(nonce);
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                // Play terminal: answer ConPTY's cursor-position query.
                if buf[..n].windows(4).any(|w| w == b"\x1b[6n") {
                    let _ = dsr_writer.lock().unwrap().send(b"\x1b[1;1R");
                }
                for seg in parser.feed(&buf[..n]) {
                    if let Segment::Prompt(p) = seg {
                        if tx.send(p).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        let next = || rx.recv_timeout(Duration::from_secs(30)).expect("no prompt marker from the shell");

        let mut shell = ShellStatus { integration: true, ..Default::default() };
        let mut st = RunnerState::default();
        let first = next();
        handle_prompt(&mut shell, &mut st, &mut *writer.lock().unwrap(), first);
        assert!(shell.ready && shell.idle);
        assert!(shell.cwd.as_ref().unwrap().join(".git").exists(), "cwd from marker: {:?}", shell.cwd);

        let steps = vec![
            step("git checkout -b 'feat/e2e'", false),
            step("git add -A", false),
            step("git commit -q -m 'it'\\''s a test'", false),
            step("git push -q -u origin 'feat/e2e'", false),
            step("git checkout -q nonexistent-branch", false),
            step("echo never-runs", false),
            step("git checkout -q main", true),
        ];
        let mut all = start(&mut shell, &mut st, &mut *writer.lock().unwrap(), meta("e2e"), steps).unwrap();
        while st.run.is_some() {
            let p = next();
            all.extend(handle_prompt(&mut shell, &mut st, &mut *writer.lock().unwrap(), p));
        }
        let _ = child.kill();

        let d = done(&all).unwrap();
        assert_eq!(d.status, RunStatus::Failed);
        assert_eq!(d.failed_step, Some(4));
        assert_ne!(d.exit_code, Some(0));
        let skipped: Vec<_> = all
            .iter()
            .filter_map(|e| match e {
                Event::Step(s) if s.state == StepState::Skipped => Some(s.index),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, vec![5]);

        // The always-step brought us back, and the branch reached the bare origin.
        assert_eq!(git::git(&repo.work, &["symbolic-ref", "--short", "HEAD"]).as_deref(), Some("main"));
        assert_eq!(
            git::git(&repo.origin, &["log", "-1", "--format=%s", "feat/e2e"]).as_deref(),
            Some("it's a test")
        );
    }
}
