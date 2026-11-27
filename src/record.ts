// Recording control bar: elapsed time, draw mode and its tools, stop and discard.
// Kadro keeps it out of the recording itself.
import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { DRAW_COLORS, DRAW_TOOLS, type DrawKey, type DrawSet, type DrawState } from "./drawtools";
import { pretty } from "./keys";

declare global {
  interface Window {
    __KADRO_REC__?: { format: "mp4" | "gif"; draw?: boolean };
  }
}

const TOOL_ICONS: Record<string, string> = {
  pen: '<svg viewBox="0 0 24 24"><path d="M4 20l4-1 11-11a2.1 2.1 0 0 0-3-3L5 16l-1 4z" /></svg>',
  marker: '<svg viewBox="0 0 24 24"><path d="M9 15l-3 3v2h4l3-3M9 15l7-11 4 4-11 7z" /></svg>',
  arrow: '<svg viewBox="0 0 24 24"><path d="M5 19L19 5M9 5h10v10" /></svg>',
  rect: '<svg viewBox="0 0 24 24"><rect x="4" y="6" width="16" height="12" rx="1" /></svg>',
};

const $ = (id: string) => document.getElementById(id)!;
const time = $("time");
const stopBtn = $("stop") as HTMLButtonElement;
const discardBtn = $("discard") as HTMLButtonElement;
const drawBtn = $("draw") as HTMLButtonElement;
const fadeBtn = $("fade") as HTMLButtonElement;
const tools = $("tools");
const isMac = navigator.userAgent.includes("Mac");

const format = window.__KADRO_REC__?.format ?? "mp4";
// Linux without a compositor can't draw on the screen: no pencil there.
if (window.__KADRO_REC__?.draw === false) drawBtn.hidden = true;
$("format").textContent = format.toUpperCase();

let started = 0;
let timer = 0;
let done = false;

function clock(ms: number): string {
  const s = Math.floor(ms / 1000);
  const mm = String(Math.floor(s / 60) % 60).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return s >= 3600 ? `${Math.floor(s / 3600)}:${mm}:${ss}` : `${mm}:${ss}`;
}

function tick() {
  time.textContent = clock(Date.now() - started);
}

function finishing(text: string) {
  done = true;
  clearInterval(timer);
  document.body.classList.remove("recording");
  time.textContent = text;
  stopBtn.disabled = true;
  discardBtn.disabled = true;
  drawBtn.disabled = true;
  setDrawing(false);
}

function begin(elapsed: number) {
  if (done || started) return;
  started = Date.now() - elapsed;
  document.body.classList.add("recording");
  stopBtn.disabled = false;
  drawBtn.disabled = false;
  void emitTo("draw", "draw-hello");
  tick();
  timer = window.setInterval(tick, 250);
}

async function stop(discard: boolean) {
  if (done) return;
  finishing(discard ? "Descartando…" : "Salvando…");
  await invoke("stop_recording", { discard });
}

let armTimer = 0;
function disarm() {
  clearTimeout(armTimer);
  discardBtn.classList.remove("confirm");
  discardBtn.title = "Descartar gravação";
}

discardBtn.addEventListener("click", () => {
  // Two clicks, so a slip doesn't throw a recording away.
  if (!discardBtn.classList.contains("confirm")) {
    discardBtn.classList.add("confirm");
    discardBtn.title = "Clique de novo para descartar";
    armTimer = window.setTimeout(disarm, 3000);
    return;
  }
  disarm();
  void stop(true);
});
stopBtn.addEventListener("click", () => void stop(false));

// ---- Drawing on the screen ----

/** Width of the bar without the drawing tools, in CSS pixels (BAR_WIDTH in Rust). */
const BAR_WIDTH = 272;
let drawing = false;
let drawState: DrawState | null = null;
const toolButtons = new Map<string, HTMLButtonElement>();
const swatches = new Map<string, HTMLButtonElement>();

const send = (change: DrawSet) => void emitTo("draw", "draw-set", change);

for (const t of DRAW_TOOLS) {
  const b = document.createElement("button");
  b.className = "icon";
  b.title = `${t.name} (${t.key})`;
  b.setAttribute("aria-label", t.name);
  b.innerHTML = TOOL_ICONS[t.tool];
  b.addEventListener("click", () => send({ tool: t.tool }));
  $("tool-buttons").append(b);
  toolButtons.set(t.tool, b);
}
DRAW_COLORS.forEach((color, i) => {
  const b = document.createElement("button");
  b.className = "swatch";
  b.title = `Cor (${i + 1})`;
  b.setAttribute("aria-label", `Cor ${i + 1}`);
  const dot = document.createElement("span");
  dot.style.background = color;
  b.append(dot);
  b.addEventListener("click", () => send({ color }));
  $("colors").append(b);
  swatches.set(color, b);
});
fadeBtn.addEventListener("click", () => send({ fade: !drawState?.fade }));
$("clear").title = "Limpar desenhos (C)";
$("clear").addEventListener("click", () => send({ clear: true }));
drawBtn.addEventListener("click", () => void invoke("set_drawing", { on: null }));

function renderDraw() {
  if (!drawState) return;
  for (const [tool, b] of toolButtons) b.classList.toggle("on", tool === drawState.tool);
  for (const [color, b] of swatches) b.classList.toggle("on", color === drawState.color);
  fadeBtn.classList.toggle("on", drawState.fade);
  fadeBtn.title = drawState.fade
    ? "Apagar sozinho: ligado, os desenhos somem após 3 s (F)"
    : "Apagar sozinho: desligado (F)";
}

function setDrawing(on: boolean) {
  if (drawing === on) return;
  drawing = on;
  document.body.classList.toggle("drawing", on);
  const width = on ? BAR_WIDTH + tools.getBoundingClientRect().width + 6 : BAR_WIDTH;
  void invoke("fit_bar", { width: Math.ceil(width) });
}

// With the focus on the bar (after a click on it), the drawing keys still work.
document.addEventListener("keydown", (e) => {
  if (!drawing) return;
  const key: DrawKey = { key: e.key, mod: e.metaKey || e.ctrlKey, shift: e.shiftKey };
  void emitTo("draw", "draw-key", key);
  e.preventDefault();
});
// Buttons must not keep the focus: this bar shouldn't steal keys from other apps.
document.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button")) e.preventDefault();
});

async function init() {
  discardBtn.title = "Descartar gravação";
  await listen<{ on: boolean }>("draw-mode", (e) => setDrawing(e.payload.on));
  await listen<DrawState>("draw-state", (e) => {
    drawState = e.payload;
    renderDraw();
  });
  await listen("recording-started", () => begin(0));
  await listen("recording-stopping", () => finishing("Salvando…"));
  // The recording may have started before this page finished loading.
  const elapsed = await invoke<number | null>("recording_elapsed");
  if (elapsed !== null) begin(elapsed);
  try {
    const s = await invoke<{ shortcut_record: string; shortcut_draw: string }>("get_settings");
    stopBtn.title = `Parar e salvar (${pretty(s.shortcut_record, isMac)})`;
    drawBtn.title = `Desenhar na tela (${pretty(s.shortcut_draw, isMac)})`;
  } catch {
    stopBtn.title = "Parar e salvar";
    drawBtn.title = "Desenhar na tela";
  }
  if (await invoke<boolean>("drawing")) setDrawing(true);
}

void init();
