import { useEffect, useRef, useState, type RefObject } from "react";
import { gitBranchList } from "../lib/ipc";
import type { BranchEntry } from "../lib/types";

interface Props {
  /** The menu plus the button that toggles it: clicks inside don't count as "outside". */
  container: RefObject<HTMLElement | null>;
  /** Checked-out branch (marked, not selectable); null when detached. */
  current: string | null;
  /** Why switching is unavailable right now (shell busy, recipe running…), or null. */
  disabledReason: string | null;
  onSwitch: (branch: string) => void;
  onClose: () => void;
}

/** Every word typed must appear in the name: "feat login" finds feature/user-login. */
function matches(name: string, query: string) {
  const n = name.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((w) => n.includes(w));
}

/** Searchable list of the repo's branches; picking one switches to it in the terminal. */
export function BranchMenu({ container, current, disabledReason, onSwitch, onClose }: Props) {
  const [branches, setBranches] = useState<BranchEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    gitBranchList()
      .then(setBranches)
      .catch((e) => setError(String(e)));
  }, []);

  // Close on a click anywhere else, or Esc. Capture phase: the terminal stops
  // mouse events from bubbling, so a click there would otherwise go unnoticed.
  // The toggle button is inside `container`, so clicking it again closes the
  // menu through its own onClick instead of closing and instantly reopening.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!container.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("mousedown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [container, onClose]);

  const shown = (branches ?? []).filter((b) => b.name !== current && matches(b.name, query));
  const canSwitch = !disabledReason;

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const pick = (b: BranchEntry | undefined) => {
    if (!b || !canSwitch) return;
    onSwitch(b.name);
    onClose();
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, shown.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      pick(shown[active]);
    }
  };

  return (
    <div className="branch-menu" role="dialog" aria-label="Switch branch">
      <input
        className="branch-search"
        placeholder="Search branches…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={onKey}
        autoFocus
        spellCheck={false}
        aria-label="Search branches"
      />
      {disabledReason && <div className="hint">{disabledReason}</div>}
      {error && <div className="error-box">{error}</div>}
      {!branches && !error && <div className="muted small branch-menu-note">Loading branches…</div>}
      {branches && (
        <ul className="branch-options" ref={list} role="listbox">
          {current && (
            <li className="branch-option current" aria-disabled title="You are on this branch">
              <span className="branch-option-name">{current}</span>
              <span className="muted small">current</span>
            </li>
          )}
          {shown.map((b, i) => (
            <li
              key={b.name}
              data-index={i}
              role="option"
              aria-selected={i === active}
              aria-disabled={!canSwitch}
              className={`branch-option${i === active ? " active" : ""}${canSwitch ? "" : " disabled"}`}
              title={b.remoteOnly ? `Only on origin: switching creates a local ${b.name} that tracks it` : b.name}
              onMouseEnter={() => setActive(i)}
              onClick={() => pick(b)}
            >
              <span className="branch-option-name">{b.name}</span>
              {b.remoteOnly && <span className="badge">origin</span>}
            </li>
          ))}
          {shown.length === 0 && (
            <li className="muted small branch-menu-note">{query ? "No branches match." : "No other branches."}</li>
          )}
        </ul>
      )}
    </div>
  );
}
