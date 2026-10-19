import { useEffect, useState } from "react";
import { repoTrust } from "../lib/ipc";
import type { PendingTrust } from "../lib/types";

interface Props {
  pending: PendingTrust;
  onClose: () => void;
  onTrusted: () => void;
}

/** Shows a repo's .branchdial.yml so the user can review it before its buttons appear. */
export function TrustDialog({ pending, onClose, onTrusted }: Props) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const trust = async () => {
    setBusy(true);
    try {
      await repoTrust(pending.hash);
      onTrusted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog-wide" role="dialog" aria-label="Review team recipes">
        <header className="dialog-header">
          <h2>{pending.changed ? "Team recipes changed" : "Review team recipes"}</h2>
          <p className="muted">
            {pending.changed
              ? "This repo's recipe file changed since you trusted it. Read the new version before its buttons come back."
              : "This repo ships its own recipe buttons. They can run any command, so read them before trusting them."}
          </p>
        </header>
        <div className="muted small selectable">{pending.file}</div>
        <pre className="snippet trust-content selectable">{pending.content}</pre>
        {error && <div className="error-box">{error}</div>}
        <footer className="dialog-footer">
          <span className="muted kbd-hint">Trust is remembered until the file changes.</span>
          <button className="btn" onClick={onClose} autoFocus>
            Not now
          </button>
          <button className="btn btn-primary" onClick={trust} disabled={busy}>
            Trust these recipes
          </button>
        </footer>
      </div>
    </div>
  );
}
