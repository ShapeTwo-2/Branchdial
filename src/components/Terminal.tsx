import { useEffect, useRef, useState } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { onPtyExit, ptyResize, ptySpawn, ptyWrite } from "../lib/ipc";
import type { SpawnInfo } from "../lib/types";

const THEME = {
  background: "#14161b",
  foreground: "#d7dae0",
  cursor: "#e5c07b",
  selectionBackground: "#3e4451",
  black: "#1e2127",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#e5c07b",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d7dae0",
  brightBlack: "#5c6370",
  brightRed: "#ef7b84",
  brightGreen: "#a9d48a",
  brightYellow: "#f0cd8c",
  brightBlue: "#72c0ff",
  brightMagenta: "#d789ee",
  brightCyan: "#67c7d3",
  brightWhite: "#ffffff",
};

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Fallback for webviews that don't grant the async clipboard API.
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

interface Props {
  onSpawned: (info: SpawnInfo) => void;
  /** Filled with a function that focuses the terminal. */
  focusRef: React.MutableRefObject<() => void>;
}

/** Only web links are acted on; `file:`, `javascript:` and the like never are. */
function webUrl(uri: string): string | null {
  try {
    const u = new URL(uri);
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : null;
  } catch {
    return null;
  }
}

export function Terminal({ onSpawned, focusRef }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const onSpawnedRef = useRef(onSpawned);
  onSpawnedRef.current = onSpawned;
  // Hyperlinks (OSC 8) show their real target, whatever text the program printed.
  const [link, setLink] = useState<{ url: string; note: string } | null>(null);

  useEffect(() => {
    const host = hostRef.current!;
    const term = new XTerm({
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, Menlo, "DejaVu Sans Mono", monospace',
      fontSize: 13,
      cursorBlink: true,
      scrollback: 10000,
      theme: THEME,
      allowProposedApi: false,
      // Replaces xterm's default, which shows a confirm() with the program's text and
      // tries window.open(). Here a link never opens anything: hovering shows the
      // real URL, clicking copies a web URL so the user can paste it in a browser.
      linkHandler: {
        allowNonHttpProtocols: false,
        hover: (_e, uri) => setLink({ url: uri, note: webUrl(uri) ? "Click to copy" : "Not a web link" }),
        leave: () => setLink(null),
        activate: (_e, uri) => {
          const url = webUrl(uri);
          if (!url) return;
          void copyText(url);
          setLink({ url, note: "Copied" });
        },
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    let exited = false;
    let disposed = false;

    // Windows-terminal style: Ctrl+C copies when there is a selection, otherwise sends ^C.
    // Ctrl+V / Ctrl+Shift+V fall through to the browser's paste event, which xterm handles.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const key = e.key.toLowerCase();
      if (e.ctrlKey && key === "c" && (e.shiftKey || term.hasSelection())) {
        if (term.hasSelection()) {
          void copyText(term.getSelection());
          term.clearSelection();
        }
        e.preventDefault();
        return false;
      }
      if (e.ctrlKey && key === "v") return false;
      return true;
    });

    const spawn = () => {
      exited = false;
      ptySpawn(term.cols, term.rows, (chunk) => term.write(chunk))
        .then((info) => {
          if (info.windowsBuild) {
            term.options.windowsPty = { backend: "conpty", buildNumber: info.windowsBuild };
          }
          onSpawnedRef.current(info);
        })
        .catch((err) => {
          term.write(`\r\n\x1b[31mFailed to start shell: ${err}\x1b[0m\r\n`);
        });
    };

    const dataSub = term.onData((data) => {
      if (exited) {
        if (data === "\r") {
          term.reset();
          spawn();
        }
        return;
      }
      void ptyWrite(data);
    });
    const resizeSub = term.onResize(({ cols, rows }) => void ptyResize(cols, rows));

    const unlistenExit = onPtyExit(({ code }) => {
      exited = true;
      term.write(`\r\n\x1b[90m[shell exited${code === null ? "" : ` with code ${code}`} — press Enter to restart]\x1b[0m\r\n`);
    });

    // Refit when the container changes size; rAF keeps it to one fit per frame.
    let frame = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!disposed && host.clientWidth > 0) fit.fit();
      });
    });
    ro.observe(host);

    focusRef.current = () => term.focus();
    spawn();
    term.focus();

    return () => {
      disposed = true;
      focusRef.current = () => {};
      ro.disconnect();
      cancelAnimationFrame(frame);
      dataSub.dispose();
      resizeSub.dispose();
      void unlistenExit.then((f) => f());
      term.dispose();
    };
  }, []);

  return (
    <div className="terminal-wrap">
      <div className="terminal-host" ref={hostRef} />
      {link && (
        <div className="link-preview" role="status">
          <span className="link-url selectable">{link.url}</span>
          <span className="muted">{link.note}</span>
        </div>
      )}
    </div>
  );
}
