import { useEffect, useMemo, useState } from "react";
import { repoTrust } from "../lib/ipc";
import { lineDiff, withContext } from "../lib/diff";
import { hasHidden, Reveal } from "../lib/hidden";
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
  // A changed file opens on what changed; the whole file is one click away.
  const diff = useMemo(
    () => (pending.previous !== null ? lineDiff(pending.previous, pending.content) : null),
    [pending.previous, pending.content],
  );
  const [whole, setWhole] = useState(diff === null);
  const hidden = hasHidden(pending.content);

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

  const rows = diff ? withContext(diff) : [];
  const onlyWhitespace = diff !== null && rows.every((r) => r.kind === "fold");

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog-wide" role="dialog" aria-label="Review team recipes">
        <header className="dialog-header">
          <h2>{pending.changed ? "Team recipes changed" : "Review team recipes"}</h2>
          <p className="muted">
            {pending.changed
              ? "This repo's recipe file changed since you trusted it. Read the changes before its buttons come back."
              : "This repo ships its own recipe buttons. They can run any command, so read them before trusting them."}
          </p>
        </header>
        <div className="muted small selectable">{pending.file}</div>
        {hidden && (
          <div className="error-box" role="alert">
            This file contains invisible or text-direction characters (shown in red as ⟨U+…⟩). They can make a
            command look different from what it runs. Don't trust it unless you know why they're there.
          </div>
        )}
        {diff && (
          <div className="trust-view-toggle">
            <button className={`btn btn-small${whole ? "" : " active"}`} onClick={() => setWhole(false)}>
              Changes
            </button>
            <button className={`btn btn-small${whole ? " active" : ""}`} onClick={() => setWhole(true)}>
              Whole file
            </button>
          </div>
        )}
        {whole || !diff ? (
          <pre className="snippet trust-content selectable">
            <Reveal text={pending.content} />
          </pre>
        ) : (
          <pre className="snippet trust-content trust-diff selectable">
            {onlyWhitespace && <div className="diff-fold">Only line endings or trailing whitespace changed.</div>}
            {rows.map((r, i) =>
              r.kind === "fold" ? (
                <div key={i} className="diff-fold">
                  ⋯ {r.count} unchanged line{r.count === 1 ? "" : "s"}
                </div>
              ) : (
                <div key={i} className={`diff-${r.kind}`}>
                  <span className="diff-sign" aria-hidden>
                    {r.kind === "add" ? "+" : r.kind === "del" ? "-" : " "}
                  </span>
                  <Reveal text={r.text} />
                </div>
              ),
            )}
          </pre>
        )}
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
