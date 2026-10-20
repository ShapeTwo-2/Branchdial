// Invisible and direction-changing characters ("Trojan Source"): text that displays
// differently from what it is. Mirrors `is_hidden_char` in src-tauri/src/recipes/plan.rs.
// Shown as a visible ⟨U+XXXX⟩ badge wherever commands, file names or recipe files
// are reviewed, so a preview can't look harmless while being something else.

import type { ReactNode } from "react";

const HIDDEN =
  /[­͏؜ᅟᅠ឴឵᠋-᠏​-‏ -‮⁠-⁯ㅤ︀-️﻿ﾠ￰-￻\u{E0000}-\u{E0FFF}]/gu;

export function hasHidden(text: string): boolean {
  HIDDEN.lastIndex = 0;
  return HIDDEN.test(text);
}

function codePoint(ch: string) {
  return `U+${ch.codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}`;
}

/** Renders `text` with every hidden character replaced by a highlighted ⟨U+XXXX⟩. */
export function Reveal({ text }: { text: string }) {
  const parts: ReactNode[] = [];
  let last = 0;
  HIDDEN.lastIndex = 0;
  for (const m of text.matchAll(HIDDEN)) {
    const i = m.index!;
    if (i > last) parts.push(text.slice(last, i));
    const cp = codePoint(m[0]);
    parts.push(
      <mark key={i} className="hidden-char" title={`Invisible or text-direction character ${cp}`}>
        ⟨{cp}⟩
      </mark>,
    );
    last = i + m[0].length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return <>{parts}</>;
}
