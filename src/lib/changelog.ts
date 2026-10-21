// CHANGELOG.md, bundled into the app at build time, split into one section per version.

import raw from "../../CHANGELOG.md?raw";

export interface Release {
  version: string;
  /** The section's markdown, without its `## <version>` line. */
  body: string;
}

/** Releases in file order (newest first). */
export function releases(text: string = raw): Release[] {
  const out: Release[] = [];
  let current: Release | null = null;
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const m = /^## +v?(\d+\.\d+\.\d+)\b/.exec(line);
    if (m) {
      current = { version: m[1], body: "" };
      out.push(current);
    } else if (/^## /.test(line) || /^# /.test(line)) {
      current = null;
    } else if (current) {
      current.body += line + "\n";
    }
  }
  return out.map((r) => ({ ...r, body: r.body.trim() }));
}

/** -1, 0 or 1, comparing X.Y.Z versions numerically. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return Math.sign(d);
  }
  return 0;
}

/** What changed after `seen` up to and including `current`; with no `seen`, just `current`. */
export function newSince(seen: string | null, current: string, all: Release[] = releases()): Release[] {
  return all.filter(
    (r) => compareVersions(r.version, current) <= 0 && (seen ? compareVersions(r.version, seen) > 0 : r.version === current),
  );
}
