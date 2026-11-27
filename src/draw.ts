// Draw overlay: a transparent window over the recorded area. In draw mode it takes the
// mouse and draws; otherwise Kadro makes it click-through and the strokes just stay
// (or fade away). Whatever is drawn here is on screen, so it is recorded.
import { invoke } from "@tauri-apps/api/core";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { drawShape, snapAngle, snapSquare, isEmpty, SIZES, type Pt, type Shape } from "./annotate";
import { DRAW_COLORS, DRAW_TOOLS, type DrawKey, type DrawSet, type DrawState } from "./drawtools";

/** Strokes start fading this long after they are drawn, when auto-fade is on… */
const FADE_DELAY = 3000;
/** …and are gone this much later. */
const FADE_TIME = 700;
const STORE = "kadro.draw";

type Stroke = { shape: Shape; at: number };

const canvas = document.getElementById("ink") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const size = SIZES[1];

let state: DrawState = { tool: "pen", color: DRAW_COLORS[0], fade: false };
try {
  const saved = JSON.parse(localStorage.getItem(STORE) ?? "null");
  if (saved && DRAW_TOOLS.some((t) => t.tool === saved.tool) && DRAW_COLORS.includes(saved.color)) {
    state = { tool: saved.tool, color: saved.color, fade: !!saved.fade };
  }
} catch {
  // No storage: start with the defaults.
}

let active = false;
let strokes: Stroke[] = [];
let current: Shape | null = null;
let start: Pt | null = null;
let shift = false;
let frame = 0;

function publish() {
  try {
    localStorage.setItem(STORE, JSON.stringify(state));
  } catch {
    // Not remembered next time; fine.
  }
  void emitTo("recorder", "draw-state", state);
}

function resize() {
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(window.innerWidth * dpr);
  canvas.height = Math.round(window.innerHeight * dpr);
  redraw();
}

/** Opacity of a finished stroke now, with auto-fade. */
function alpha(s: Stroke, now: number): number {
  if (!state.fade) return 1;
  return 1 - Math.min(Math.max((now - s.at - FADE_DELAY) / FADE_TIME, 0), 1);
}

function redraw() {
  frame = 0;
  const now = performance.now();
  if (state.fade) strokes = strokes.filter((s) => alpha(s, now) > 0);
  const dpr = window.devicePixelRatio || 1;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  for (const s of strokes) {
    ctx.globalAlpha = alpha(s, now);
    drawShape(ctx, s.shape);
  }
  ctx.globalAlpha = 1;
  if (current) drawShape(ctx, current);
  // Keep animating only while something is fading; otherwise the page sits idle.
  if (state.fade && strokes.length) schedule(strokes.some((s) => now - s.at >= FADE_DELAY) ? 0 : nextFade(now));
}

function nextFade(now: number): number {
  return Math.max(0, Math.min(...strokes.map((s) => s.at + FADE_DELAY - now)));
}

let timer = 0;
function schedule(delay = 0) {
  if (delay > 0) {
    clearTimeout(timer);
    timer = window.setTimeout(() => schedule(0), delay);
    return;
  }
  if (!frame) frame = requestAnimationFrame(redraw);
}

function point(e: PointerEvent): Pt {
  return { x: e.clientX, y: e.clientY };
}

function shapeTo(b: Pt): void {
  if (!current || !start || !("b" in current)) return;
  if (current.kind === "rect") current.b = shift ? snapSquare(start, b) : b;
  else current.b = shift ? snapAngle(start, b) : b;
}

function finish() {
  if (current && !isEmpty(current)) {
    // With auto-fade, a stroke's clock starts when the mouse is released.
    strokes.push({ shape: current, at: performance.now() });
  }
  current = null;
  start = null;
  schedule();
}

canvas.addEventListener("pointerdown", (e) => {
  if (!active || e.button !== 0) return;
  canvas.setPointerCapture(e.pointerId);
  const p = point(e);
  start = p;
  shift = e.shiftKey;
  const { tool, color } = state;
  current =
    tool === "pen" || tool === "marker"
      ? { kind: tool, color, width: tool === "pen" ? size.stroke : size.marker, pts: [p] }
      : { kind: tool, color, width: size.stroke, a: p, b: p };
  schedule();
});

canvas.addEventListener("pointermove", (e) => {
  if (!current) return;
  shift = e.shiftKey;
  // Coalesced events keep fast strokes smooth.
  const events = e.getCoalescedEvents?.() ?? [e];
  if ("pts" in current) for (const ev of events.length ? events : [e]) current.pts.push(point(ev));
  else shapeTo(point(e));
  schedule();
});

canvas.addEventListener("pointerup", finish);
canvas.addEventListener("pointercancel", finish);

function apply(change: DrawSet) {
  const { clear, undo, ...rest } = change;
  if (clear) {
    strokes = [];
    current = null;
  }
  if (undo) strokes.pop();
  if (Object.keys(rest).length) {
    const fadeOn = rest.fade === true && !state.fade;
    state = { ...state, ...rest };
    // Turning auto-fade on gives what is already drawn its full time on screen.
    if (fadeOn) {
      const now = performance.now();
      strokes.forEach((s) => (s.at = now));
    }
    publish();
  }
  schedule();
}

function onKey(k: DrawKey): boolean {
  const key = k.key.toLowerCase();
  if (k.mod && key === "z") {
    apply({ undo: true });
    return true;
  }
  if (k.mod) return false;
  if (key === "escape") {
    void invoke("set_drawing", { on: false });
    return true;
  }
  const tool = DRAW_TOOLS.find((t) => t.key.toLowerCase() === key);
  if (tool) {
    apply({ tool: tool.tool });
    return true;
  }
  const n = Number(key);
  if (n >= 1 && n <= DRAW_COLORS.length) {
    apply({ color: DRAW_COLORS[n - 1] });
    return true;
  }
  if (key === "f") {
    apply({ fade: !state.fade });
    return true;
  }
  if (key === "c" || key === "delete" || key === "backspace") {
    apply({ clear: true });
    return true;
  }
  return false;
}

document.addEventListener("keydown", (e) => {
  if (onKey({ key: e.key, mod: e.metaKey || e.ctrlKey, shift: e.shiftKey })) e.preventDefault();
});

function setActive(on: boolean) {
  active = on;
  document.body.classList.toggle("active", on);
  if (!on && current) finish();
}

window.addEventListener("resize", resize);

async function init() {
  resize();
  await listen<{ on: boolean }>("draw-mode", (e) => setActive(e.payload.on));
  await listen<DrawSet>("draw-set", (e) => apply(e.payload));
  await listen<DrawKey>("draw-key", (e) => void onKey(e.payload));
  // The bar asks for the current tools once it is ready.
  await listen("draw-hello", () => publish());
  setActive(await invoke<boolean>("drawing"));
  publish();
  await getCurrentWindow().show();
}

void init();
