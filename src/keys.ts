// Shortcut labels shared by the preferences window and the gallery.

const MAC_SYMBOLS: Record<string, string> = {
  Control: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Command: "⌘",
  Super: "⌘",
};
const ORDER = ["Control", "Alt", "Shift", "Command", "Super"];

/** Turns "CommandOrControl+Shift+Digit2" into "⌘ ⇧ 2" (macOS) or "Ctrl + Shift + 2". */
export function pretty(accelerator: string, isMac: boolean): string {
  const parts = accelerator.split("+").map((p) => {
    const lower = p.toLowerCase();
    if (lower === "commandorcontrol" || lower === "cmdorctrl") return isMac ? "Command" : "Control";
    if (lower === "cmd" || lower === "command" || lower === "meta") return "Command";
    if (lower === "ctrl" || lower === "control") return "Control";
    if (lower === "option" || lower === "alt") return "Alt";
    if (lower === "shift") return "Shift";
    if (lower === "super") return "Super";
    return p;
  });
  const mods = ORDER.filter((m) => parts.includes(m));
  const key = parts.filter((p) => !ORDER.includes(p)).map((k) => prettyKey(k, isMac)).join("");
  if (isMac) return [...mods.map((m) => MAC_SYMBOLS[m]), key].join(" ");
  const names: Record<string, string> = { Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win" };
  return [...mods.map((m) => names[m] ?? m), key].join(" + ");
}

function prettyKey(code: string, isMac: boolean): string {
  const m = /^(?:Key|Digit|Numpad)(.+)$/.exec(code);
  if (m) return m[1];
  const names: Record<string, string> = {
    Space: isMac ? "Espaço" : "Space",
    ArrowUp: "↑",
    ArrowDown: "↓",
    ArrowLeft: "←",
    ArrowRight: "→",
    PrintScreen: "Print Screen",
    Backquote: "`",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
  };
  return names[code] ?? code;
}
