import type { GitInfo, RepoStatus } from "../lib/types";

interface Props {
  /** null while the startup check runs. */
  git: GitInfo | null;
  status: RepoStatus | null;
  onRefresh: () => void;
}

function basename(path: string) {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path;
}

const OPERATION_LABEL = {
  merge: "Merge",
  rebase: "Rebase",
  "cherry-pick": "Cherry-pick",
  revert: "Revert",
} as const;

function plural(n: number, word: string) {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export function StatusBar({ git, status, onRefresh }: Props) {
  if (git && !git.install) {
    return (
      <section className="sidebar-section status status-error" role="alert">
        <div className="status-title">Git not found</div>
        <div className="status-hint selectable">{git.installHint}</div>
      </section>
    );
  }

  if (!status) {
    return (
      <section className="sidebar-section status">
        <span className="muted">Reading repository…</span>
      </section>
    );
  }

  if (!status.root) {
    return (
      <section className="sidebar-section status" title={status.cwd ?? undefined}>
        <div className="status-top">
          <span className="muted">Not in a git repository</span>
          <RefreshButton onClick={onRefresh} />
        </div>
        <div className="status-hint">cd into one in the terminal.</div>
      </section>
    );
  }

  const changed = status.staged + status.modified + status.untracked + status.conflicted;
  const breakdown = [
    status.staged && `${status.staged} staged`,
    status.modified && `${status.modified} modified`,
    status.untracked && `${status.untracked} untracked`,
    status.conflicted && `${status.conflicted} conflicted`,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <section className="sidebar-section status" title={status.root}>
      <div className="status-top">
        <span className="repo-name">{basename(status.root)}</span>
        <RefreshButton onClick={onRefresh} />
      </div>

      <div className="status-branch">
        <span className="branch-glyph" aria-hidden>
          ⎇
        </span>
        {status.branch ? (
          <span className="branch-name">{status.branch}</span>
        ) : (
          <span className="branch-name detached" title="HEAD is not on a branch">
            detached at {status.head ?? "?"}
          </span>
        )}
      </div>

      <div className="status-row">
        {status.upstream ? (
          status.ahead === 0 && status.behind === 0 ? (
            <span className="muted" title={`Tracking ${status.upstream}`}>
              In sync with {status.upstream}
            </span>
          ) : (
            <span title={`Compared with ${status.upstream}`}>
              {status.ahead > 0 && (
                <span className="ahead" title={`${plural(status.ahead, "commit")} to push`}>
                  ↑{status.ahead}
                </span>
              )}
              {status.behind > 0 && (
                <span className="behind" title={`${plural(status.behind, "commit")} to pull`}>
                  ↓{status.behind}
                </span>
              )}
              <span className="muted"> {status.upstream}</span>
            </span>
          )
        ) : status.branch ? (
          <span className="muted">No upstream (not pushed yet)</span>
        ) : null}
      </div>

      <div className="status-row">
        {changed === 0 ? (
          <span className="clean">✓ Clean</span>
        ) : (
          <span className="dirty" title={breakdown}>
            ● {plural(changed, "changed file")}
            <span className="muted"> ({breakdown})</span>
          </span>
        )}
      </div>

      {status.operation && (
        <div className="status-warning" role="alert">
          ⚠ {OPERATION_LABEL[status.operation]} in progress
          {status.conflicted > 0 && <> · {plural(status.conflicted, "conflicted file")}</>}
        </div>
      )}
    </section>
  );
}

function RefreshButton({ onClick }: { onClick: () => void }) {
  return (
    <button className="icon-btn" title="Refresh status" onClick={onClick}>
      ⟳
    </button>
  );
}
