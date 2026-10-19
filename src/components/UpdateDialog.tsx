import { useEffect, useState } from "react";
import { updateCheck, updateInstall, type Update } from "../lib/ipc";

interface Props {
  /** An update found by the startup check, or null to check now ("Check for updates"). */
  found: Update | null;
  /** A recipe is running: updating would close the shell under it. */
  running: boolean;
  onSkip: (version: string) => void;
  onClose: () => void;
}

type Phase =
  | { kind: "checking" }
  | { kind: "latest" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; done: number; total: number | null }
  | { kind: "installing"; update: Update }
  | { kind: "error"; message: string; update: Update | null };

function mb(bytes: number) {
  return (bytes / 1024 / 1024).toFixed(1);
}

/** Asks before updating, then shows download progress until the app restarts. */
export function UpdateDialog({ found, running, onSkip, onClose }: Props) {
  const [phase, setPhase] = useState<Phase>(found ? { kind: "available", update: found } : { kind: "checking" });
  const busy = phase.kind === "downloading" || phase.kind === "installing";

  useEffect(() => {
    if (found) return;
    updateCheck()
      .then((u) => setPhase(u ? { kind: "available", update: u } : { kind: "latest" }))
      .catch((e) => setPhase({ kind: "error", message: String(e), update: null }));
  }, [found]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const install = async (update: Update) => {
    setPhase({ kind: "downloading", update, done: 0, total: null });
    try {
      await updateInstall(update, (done, total) =>
        setPhase(done === total ? { kind: "installing", update } : { kind: "downloading", update, done, total }),
      );
    } catch (e) {
      setPhase({ kind: "error", message: String(e), update });
    }
  };

  const update = "update" in phase ? phase.update : null;

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div className="dialog" role="dialog" aria-label="Update Branchdial">
        <header className="dialog-header">
          <h2>
            {phase.kind === "checking" && "Checking for updates…"}
            {phase.kind === "latest" && "Branchdial is up to date"}
            {phase.kind === "error" && (update ? "The update failed" : "Could not check for updates")}
            {update && phase.kind !== "error" && `Branchdial ${update.version} is available`}
          </h2>
          {update && phase.kind === "available" && (
            <p className="muted">You have {update.currentVersion}. Update now? Branchdial restarts when it's done.</p>
          )}
        </header>

        {phase.kind === "available" && update?.body && <pre className="snippet update-notes selectable">{update.body}</pre>}

        {phase.kind === "downloading" && (
          <div className="update-progress">
            <progress value={phase.total ? phase.done : undefined} max={phase.total ?? undefined} />
            <span className="muted small">
              {phase.total ? `${mb(phase.done)} / ${mb(phase.total)} MB` : `${mb(phase.done)} MB`}
            </span>
          </div>
        )}
        {phase.kind === "installing" && <p className="muted">Installing… Branchdial will restart.</p>}
        {phase.kind === "error" && <div className="error-box selectable">{phase.message}</div>}

        <footer className="dialog-footer">
          {phase.kind === "available" && running && <span className="muted kbd-hint">Finish the running recipe first.</span>}
          {phase.kind === "available" && update && (
            <>
              <button className="btn" onClick={() => onSkip(update.version)}>
                Skip this version
              </button>
              <button className="btn" onClick={onClose} autoFocus>
                Later
              </button>
              <button className="btn btn-primary" onClick={() => install(update)} disabled={running}>
                Update now
              </button>
            </>
          )}
          {phase.kind === "error" && update && (
            <button className="btn btn-primary" onClick={() => install(update)} disabled={running}>
              Try again
            </button>
          )}
          {(phase.kind === "checking" || phase.kind === "latest" || phase.kind === "error") && (
            <button className="btn" onClick={onClose} autoFocus>
              Close
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
