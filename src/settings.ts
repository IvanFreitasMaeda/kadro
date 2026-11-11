// Preferences window: record global shortcuts and pick the captures folder.
import { invoke } from "@tauri-apps/api/core";

type Kind = "area" | "full";
type Settings = {
  shortcut_area: string;
  shortcut_full: string;
  save_dir: string;
  platform: string;
};

const status = document.getElementById("status")!;
const dirEl = document.getElementById("dir")!;
const buttons = new Map<Kind, HTMLButtonElement>();
document.querySelectorAll<HTMLButtonElement>(".shortcut").forEach((b) => {
  buttons.set(b.dataset.kind as Kind, b);
});

let isMac = navigator.userAgent.includes("Mac");
let recording: Kind | null = null;
const values: Record<Kind, string> = { area: "", full: "" };
const DEFAULT_HINT = status.innerHTML;

const MAC_SYMBOLS: Record<string, string> = {
  Control: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Command: "⌘",
  Super: "⌘",
};
const ORDER = ["Control", "Alt", "Shift", "Command", "Super"];

/** Turns "CommandOrControl+Shift+Digit2" into "⌘ ⇧ 2" (macOS) or "Ctrl + Shift + 2". */
function pretty(accelerator: string): string {
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
  const key = parts.filter((p) => !ORDER.includes(p)).map(prettyKey).join("");
  if (isMac) return [...mods.map((m) => MAC_SYMBOLS[m]), key].join(" ");
  const names: Record<string, string> = { Control: "Ctrl", Alt: "Alt", Shift: "Shift", Super: "Win" };
  return [...mods.map((m) => names[m] ?? m), key].join(" + ");
}

function prettyKey(code: string): string {
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

/** Builds an accelerator from a key press, or null if it isn't a usable shortcut yet. */
function accelerator(e: KeyboardEvent): string | null {
  if (/^(Shift|Control|Alt|Meta|OS)(Left|Right)?$/.test(e.code)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Control");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push(isMac ? "Command" : "Super");
  const standalone = /^F\d{1,2}$/.test(e.code) || e.code === "PrintScreen";
  if (mods.length === 0 && !standalone) return null;
  return [...mods, e.code].join("+");
}

function showStatus(text: string, kind: "error" | "ok" | "" = "") {
  status.textContent = text;
  status.className = `hint ${kind}`;
}

function render() {
  for (const [kind, button] of buttons) {
    button.classList.toggle("recording", recording === kind);
    button.textContent = recording === kind ? "Pressione as teclas…" : pretty(values[kind]);
  }
}

async function startRecording(kind: Kind) {
  if (recording) await stopRecording();
  recording = kind;
  status.innerHTML = DEFAULT_HINT;
  status.className = "hint";
  render();
  await invoke("pause_shortcuts", { paused: true });
}

async function stopRecording() {
  recording = null;
  render();
  await invoke("pause_shortcuts", { paused: false });
}

async function save(kind: Kind, value: string | null) {
  try {
    values[kind] = await invoke<string>("set_shortcut", { kind, accelerator: value });
    showStatus(`Atalho salvo: ${pretty(values[kind])}`, "ok");
  } catch (err) {
    showStatus(String(err), "error");
  }
  render();
}

document.addEventListener("keydown", async (e) => {
  if (!recording) return;
  e.preventDefault();
  if (e.key === "Escape" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
    await stopRecording();
    status.innerHTML = DEFAULT_HINT;
    status.className = "hint";
    return;
  }
  const value = accelerator(e);
  if (!value) {
    if (!/^(Shift|Control|Alt|Meta|OS)/.test(e.code)) {
      showStatus("Use pelo menos uma tecla modificadora (Ctrl, Shift, Alt ou ⌘).", "error");
    }
    return;
  }
  const kind = recording;
  recording = null;
  await save(kind, value);
  await invoke("pause_shortcuts", { paused: false });
});

for (const [kind, button] of buttons) {
  button.addEventListener("click", () => void startRecording(kind));
}
document.querySelectorAll<HTMLButtonElement>("[data-reset]").forEach((b) => {
  b.addEventListener("click", async () => {
    if (recording) await stopRecording();
    await save(b.dataset.reset as Kind, null);
  });
});
document.getElementById("choose-dir")!.addEventListener("click", async () => {
  try {
    const dir = await invoke<string | null>("choose_save_dir");
    if (dir) {
      dirEl.textContent = `\u200e${dir}\u200e`;
      dirEl.title = dir;
      showStatus("Pasta de capturas atualizada.", "ok");
    }
  } catch (err) {
    showStatus(String(err), "error");
  }
});
window.addEventListener("blur", () => {
  if (recording) void stopRecording();
});

async function init() {
  const s = await invoke<Settings>("get_settings");
  isMac = s.platform === "macos";
  values.area = s.shortcut_area;
  values.full = s.shortcut_full;
  dirEl.textContent = `\u200e${s.save_dir}\u200e`;
  dirEl.title = s.save_dir;
  render();
}

void init();
