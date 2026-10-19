import { useEffect, useState } from "react";
import { conflictAbort, conflictContinue, conflictFiles, recipeCancel } from "../lib/ipc";
import type { PlannedStep, RepoStatus, RunDone, StepState } from "../lib/types";

export interface RunView {
  name: string;
  steps: PlannedStep[];
  states: StepState[];
  exitCodes: (number | null)[];
  notes: (string | null)[];
  done: RunDone | null;
}

const ICON: Record<StepState, string> = {
  pending: "○",
  running: "◐",
  ok: "✓",
  failed: "✗",
  skipped: "–",
};

interface Props {
  run: RunView;
  /** Latest repo status; conflict files are re-read whenever it changes. */
  status: RepoStatus | null;
  onDismiss: () => void;
}

export function RunPanel({ run, status, onDismiss }: Props) {
  const { done } = run;

  // Successful runs get out of the way on their own.
  useEffect(() => {
    if (done?.status !== "ok") return;
    const t = setTimeout(onDismiss, 4000);
    return () => clearTimeout(t);
  }, [done, onDismiss]);

  let title: string;
  let tone: string;
  if (!done) {
    title = `Running: ${run.name}`;
    tone = "running";
  } else if (done.status === "ok") {
    title = `Done: ${run.name}`;
    tone = "ok";
  } else if (done.status === "cancelled") {
    title = `Cancelled: ${run.name}`;
    tone = "warn";
  } else if (done.failedStep !== null) {
    title = `${run.name} stopped: step ${done.failedStep + 1} failed (exit ${done.exitCode})`;
    tone = "failed";
  } else {
    title = `${run.name} stopped`;
    tone = "failed";
  }

  return (
    <section className={`run-panel run-${tone}`} aria-live="polite">
      <header className="run-header">
        <span className="run-title">{title}</span>
        {!done ? (
          <button className="btn btn-small" onClick={() => void recipeCancel()} title="Sends Ctrl+C to the running step">
            Stop
          </button>
        ) : (
          <button className="btn btn-small" onClick={onDismiss}>
            Dismiss
          </button>
        )}
      </header>
      <ol className="run-steps">
        {run.steps.map((s, i) => (
          <li key={i} className={`step step-${run.states[i]}`}>
            <span className="step-icon">{ICON[run.states[i]]}</span>
            <code>{s.command}</code>
            {run.states[i] === "failed" && run.exitCodes[i] !== null && (
              <span className="step-note">exit {run.exitCodes[i]}</span>
            )}
            {run.notes[i] && <span className="step-note">{run.notes[i]}</span>}
          </li>
        ))}
      </ol>
      {done?.message && <div className="run-message">{done.message}</div>}
      {done?.operation && status?.operation && <ConflictActions done={done} status={status} />}
    </section>
  );
}

/** Resolve / Abort & go back for a recipe stopped by a merge or rebase conflict. */
function ConflictActions({ done, status }: { done: RunDone; status: RepoStatus }) {
  const [resolving, setResolving] = useState(false);
  const [files, setFiles] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const op = status.operation ?? done.operation!;

  // Re-read the conflicted files whenever the status changes (e.g. after `git add`).
  useEffect(() => {
    if (!resolving) return;
    conflictFiles()
      .then(setFiles)
      .catch((e) => setError(String(e)));
  }, [resolving, status]);

  const act = (f: () => Promise<void>) => {
    setError(null);
    f().catch((e) => setError(String(e)));
  };

  const continueLabel =
    done.remaining > 0
      ? `Continue recipe (${done.remaining} more step${done.remaining === 1 ? "" : "s"})`
      : `Finish the ${op}`;

  return (
    <div className="conflict-box">
      <div className="conflict-title">
        ⚠ The {op} stopped with conflicts{status.conflicted > 0 ? ` in ${status.conflicted} file${status.conflicted === 1 ? "" : "s"}` : ""}.
      </div>
      <div className="conflict-actions">
        <button className="btn btn-small" onClick={() => setResolving((r) => !r)} aria-expanded={resolving}>
          Resolve
        </button>
        <button
          className="btn btn-small"
          onClick={() => act(conflictAbort)}
          title={`git ${op} --abort${done.startBranch ? `, then git checkout ${done.startBranch}` : ""}`}
        >
          Abort &amp; go back{done.startBranch ? ` to ${done.startBranch}` : ""}
        </button>
      </div>
      {resolving && (
        <div className="resolve-help">
          {files === null ? (
            <span className="muted">Looking for conflicts…</span>
          ) : files.length > 0 ? (
            <>
              <p>
                Open each file, keep the right parts between the <code>{"<<<<<<<"}</code>, <code>=======</code> and{" "}
                <code>{">>>>>>>"}</code> markers, then run <code>git add &lt;file&gt;</code> in the terminal:
              </p>
              <ul className="conflict-files selectable">
                {files.map((f) => (
                  <li key={f}>
                    <code>{f}</code>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p>All conflicts are resolved and staged.</p>
          )}
          <button
            className="btn btn-small btn-primary"
            disabled={files === null || files.length > 0}
            onClick={() => act(conflictContinue)}
          >
            {continueLabel}
          </button>
        </div>
      )}
      {error && <div className="error-box">{error}</div>}
    </div>
  );
}
