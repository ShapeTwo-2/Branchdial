// Line diff for reviewing a changed .branchdial.yml (longest common subsequence).
// Recipe files are small; above MAX_CELLS the caller shows the whole file instead.

export type DiffLine = { kind: "same" | "add" | "del"; text: string };

const MAX_CELLS = 4_000_000;

export function lineDiff(before: string, after: string): DiffLine[] | null {
  const a = before.replace(/\r\n/g, "\n").split("\n");
  const b = after.replace(/\r\n/g, "\n").split("\n");
  const n = a.length;
  const m = b.length;
  if (n * m > MAX_CELLS) return null;
  // lcs[i][j] = length of the LCS of a[i..] and b[j..]
  const lcs = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ kind: "del", text: a[i++] });
    } else {
      out.push({ kind: "add", text: b[j++] });
    }
  }
  while (i < n) out.push({ kind: "del", text: a[i++] });
  while (j < m) out.push({ kind: "add", text: b[j++] });
  return out;
}

/** Changed lines with `context` unchanged lines around them; longer unchanged runs fold. */
export type DiffRow = DiffLine | { kind: "fold"; count: number };

export function withContext(lines: DiffLine[], context = 3): DiffRow[] {
  const keep = lines.map(() => false);
  lines.forEach((l, i) => {
    if (l.kind === "same") return;
    for (let k = Math.max(0, i - context); k <= Math.min(lines.length - 1, i + context); k++) keep[k] = true;
  });
  const rows: DiffRow[] = [];
  let folded = 0;
  lines.forEach((l, i) => {
    if (keep[i]) {
      if (folded) rows.push({ kind: "fold", count: folded });
      folded = 0;
      rows.push(l);
    } else {
      folded++;
    }
  });
  if (folded) rows.push({ kind: "fold", count: folded });
  return rows;
}
