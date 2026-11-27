// Preferences window: launch at login, record global shortcuts, pick the captures folder and the
// history size.
import { invoke } from "@tauri-apps/api/core";
import { pretty as prettyFor } from "./keys";

type Kind = "area" | "full" | "gallery" | "record" | "text" | "clipboard" | "draw";
type Settings = {
  shortcut_area: string;
  shortcut_full: string;
  shortcut_gallery: string;
  shortcut_record: string;
  shortcut_text: string;
  shortcut_clipboard: string;
  shortcut_draw: string;
  history_limit: number;
  clip_limit: number;
  clip_days: number;
  save_dir: string;
  autostart: boolean;
  platform: string;
};

const status = document.getElementById("status")!;
const dirEl = document.getElementById("dir")!;
const buttons = new Map<Kind, HTMLButtonElement>();
document.querySelectorAll<HTMLButtonElement>(".shortcut").forEach((b) => {
  buttons.set(b.dataset.kind as Kind, b);
});

let isMac = navigator.userAgent.includes("Mac");
const pretty = (accelerator: string) => prettyFor(accelerator, isMac);
let recording: Kind | null = null;
const values: Record<Kind, string> = { area: "", full: "", gallery: "", record: "", text: "", clipboard: "", draw: "" };
const autostartEl = document.getElementById("autostart") as HTMLInputElement;
const historyEl = document.getElementById("history-limit") as HTMLSelectElement;
const clipLimitEl = document.getElementById("clip-limit") as HTMLSelectElement;
const clipDaysEl = document.getElementById("clip-days") as HTMLSelectElement;
const clipClearEl = document.getElementById("clip-clear") as HTMLButtonElement;
const DEFAULT_HINT = status.innerHTML;

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
historyEl.addEventListener("change", async () => {
  const limit = Number(historyEl.value);
  try {
    await invoke("set_history_limit", { limit });
    showStatus(
      limit === 0 ? "Histórico desativado e apagado." : `O histórico guarda as últimas ${limit} capturas.`,
      "ok",
    );
  } catch (err) {
    showStatus(String(err), "error");
  }
});
async function saveRetention() {
  const limit = Number(clipLimitEl.value);
  const days = Number(clipDaysEl.value);
  clipDaysEl.disabled = limit === 0;
  try {
    await invoke("set_clip_retention", { limit, days });
    const span = days === 0 ? "" : days === 1 ? " por 1 dia" : ` por ${days} dias`;
    showStatus(
      limit === 0
        ? "Histórico da área de transferência desativado (os fixados ficam)."
        : `A área de transferência guarda os últimos ${limit} itens${span}.`,
      "ok",
    );
  } catch (err) {
    showStatus(String(err), "error");
  }
}
clipLimitEl.addEventListener("change", () => void saveRetention());
clipDaysEl.addEventListener("change", () => void saveRetention());
let clearArmed = 0;
clipClearEl.addEventListener("click", async () => {
  // Two clicks: the first one only arms the button.
  if (!clearArmed) {
    clipClearEl.textContent = "Confirmar";
    clipClearEl.classList.add("danger");
    clearArmed = window.setTimeout(() => {
      clearArmed = 0;
      clipClearEl.textContent = "Apagar";
      clipClearEl.classList.remove("danger");
    }, 3000);
    return;
  }
  clearTimeout(clearArmed);
  clearArmed = 0;
  clipClearEl.textContent = "Apagar";
  clipClearEl.classList.remove("danger");
  try {
    await invoke("clip_clear", { pinned: true });
    showStatus("Histórico da área de transferência apagado.", "ok");
  } catch (err) {
    showStatus(String(err), "error");
  }
});
autostartEl.addEventListener("change", async () => {
  try {
    autostartEl.checked = await invoke<boolean>("set_autostart", { enabled: autostartEl.checked });
    showStatus(
      autostartEl.checked
        ? "O Kadro vai abrir sozinho, na bandeja, quando você entrar no sistema."
        : "O Kadro não abre mais junto com o sistema.",
      "ok",
    );
  } catch (err) {
    autostartEl.checked = !autostartEl.checked;
    showStatus(String(err), "error");
  }
});
window.addEventListener("blur", () => {
  if (recording) void stopRecording();
});

document.getElementById("support")!.addEventListener("click", () => {
  void invoke("open_support_page");
});

async function init() {
  const s = await invoke<Settings>("get_settings");
  isMac = s.platform === "macos";
  values.area = s.shortcut_area;
  values.full = s.shortcut_full;
  values.gallery = s.shortcut_gallery;
  values.record = s.shortcut_record;
  values.text = s.shortcut_text;
  values.clipboard = s.shortcut_clipboard;
  values.draw = s.shortcut_draw;
  for (const [el, value, label] of [
    [clipLimitEl, s.clip_limit, `Últimos ${s.clip_limit}`],
    [clipDaysEl, s.clip_days, `por ${s.clip_days} dias`],
  ] as const) {
    // A value set by hand in config.json still shows up in the list.
    if (![...el.options].some((o) => Number(o.value) === value)) el.add(new Option(label, String(value)));
    el.value = String(value);
  }
  clipDaysEl.disabled = s.clip_limit === 0;
  // A limit set by hand in config.json still shows up in the list.
  if (![...historyEl.options].some((o) => Number(o.value) === s.history_limit)) {
    historyEl.add(new Option(`Últimas ${s.history_limit}`, String(s.history_limit)));
  }
  historyEl.value = String(s.history_limit);
  autostartEl.checked = s.autostart;
  dirEl.textContent = `\u200e${s.save_dir}\u200e`;
  dirEl.title = s.save_dir;
  render();
}

void init();
