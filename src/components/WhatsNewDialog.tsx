import { useEffect, type ReactNode } from "react";
import type { Release } from "../lib/changelog";

interface Props {
  releases: Release[];
  /** "updated": shown after an update; "all": opened from the sidebar. */
  mode: "updated" | "all";
  onClose: () => void;
}

/** `**bold**` and `` `code` `` inside a line, as React elements (never HTML strings). */
function inline(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  const re = /\*\*(.+?)\*\*|`([^`]+)`/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > last) parts.push(text.slice(last, m.index));
    parts.push(m[1] !== undefined ? <strong key={m.index}>{m[1]}</strong> : <code key={m.index}>{m[2]}</code>);
    last = m.index! + m[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

/** The subset CHANGELOG.md uses: `### headings`, `- bullets` (wrapped lines indented), paragraphs. */
function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let bullets: string[] = [];
  let para: string[] = [];
  const flush = () => {
    if (bullets.length) {
      blocks.push(
        <ul key={blocks.length}>
          {bullets.map((b, i) => (
            <li key={i}>{inline(b)}</li>
          ))}
        </ul>,
      );
      bullets = [];
    }
    if (para.length) {
      blocks.push(<p key={blocks.length}>{inline(para.join(" "))}</p>);
      para = [];
    }
  };
  for (const line of text.split("\n")) {
    if (/^###\s/.test(line)) {
      flush();
      blocks.push(<h4 key={blocks.length}>{inline(line.replace(/^###\s+/, ""))}</h4>);
    } else if (/^\s*[-*]\s/.test(line)) {
      if (para.length) flush();
      bullets.push(line.replace(/^\s*[-*]\s+/, ""));
    } else if (!line.trim()) {
      flush();
    } else if (bullets.length && /^\s/.test(line)) {
      bullets[bullets.length - 1] += " " + line.trim(); // a wrapped bullet
    } else {
      if (bullets.length) flush();
      para.push(line.trim());
    }
  }
  flush();
  return <>{blocks}</>;
}

export function WhatsNewDialog({ releases, mode, onClose }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const latest = releases[0]?.version;
  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog dialog-wide" role="dialog" aria-label="What's new in Branchdial">
        <header className="dialog-header">
          <h2>{mode === "updated" && latest ? `Branchdial updated to ${latest}` : "What's new in Branchdial"}</h2>
          {mode === "updated" && releases.length > 1 && (
            <p className="muted">Changes in the {releases.length} versions since you last used Branchdial.</p>
          )}
        </header>
        <div className="whats-new selectable">
          {releases.map((r) => (
            <section key={r.version}>
              {(releases.length > 1 || mode === "all") && <h3>{r.version}</h3>}
              <Markdown text={r.body} />
            </section>
          ))}
          {releases.length === 0 && <p className="muted">No notes for this version.</p>}
        </div>
        <footer className="dialog-footer">
          <button className="btn btn-primary" onClick={onClose} autoFocus>
            Got it
          </button>
        </footer>
      </div>
    </div>
  );
}
