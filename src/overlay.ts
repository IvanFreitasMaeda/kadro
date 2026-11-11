// Capture overlay: shows the frozen screenshot of one monitor and lets the user
// select an area, then copy it or save it. One overlay window exists per monitor.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Rect = { x: number; y: number; w: number; h: number };
type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
type Drag =
  | { kind: "create"; ox: number; oy: number }
  | { kind: "move"; ox: number; oy: number; start: Rect }
  | { kind: "resize"; handle: Handle; start: Rect };

const MIN_SIZE = 4;
// Injected by the Rust side when it creates the overlay window.
declare global {
  interface Window {
    __KADRO__?: { monitor: number; nonce: number; full: boolean };
  }
}
const { monitor, nonce, full: startFull } = window.__KADRO__ ?? { monitor: 0, nonce: 0, full: false };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const shot = $<HTMLImageElement>("shot");
const dim = $("dim");
const selEl = $("sel");
const sizeEl = $("size");
const hint = $("hint");
const toolbar = $("toolbar");
const toast = $("toast");

let sel: Rect | null = null;
let drag: Drag | null = null;
let busy = false;
/** True while the selection is the whole screen, so it follows window resizes. */
let fullScreen = false;

const vw = () => window.innerWidth;
const vh = () => window.innerHeight;
const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/** Converts the selection from CSS pixels to screenshot pixels. */
function toImageRect(r: Rect) {
  const sx = shot.naturalWidth / vw();
  const sy = shot.naturalHeight / vh();
  const x = Math.round(r.x * sx);
  const y = Math.round(r.y * sy);
  return {
    x,
    y,
    width: Math.max(1, Math.min(Math.round((r.x + r.w) * sx), shot.naturalWidth) - x),
    height: Math.max(1, Math.min(Math.round((r.y + r.h) * sy), shot.naturalHeight) - y),
  };
}

function render() {
  const has = sel !== null && sel.w > 0 && sel.h > 0;
  dim.hidden = has;
  selEl.hidden = !has;
  sizeEl.hidden = !has;
  hint.hidden = has || drag !== null;
  toolbar.hidden = !has || drag !== null;
  if (!has || !sel) return;

  Object.assign(selEl.style, {
    left: `${sel.x}px`,
    top: `${sel.y}px`,
    width: `${sel.w}px`,
    height: `${sel.h}px`,
  });

  const img = toImageRect(sel);
  sizeEl.textContent = `${img.width} × ${img.height}`;
  const labelTop = sel.y - 26 >= 0 ? sel.y - 26 : sel.y + 6;
  sizeEl.style.left = `${clamp(sel.x, 4, vw() - 110)}px`;
  sizeEl.style.top = `${labelTop}px`;

  if (!toolbar.hidden) placeToolbar(sel);
}

/** Puts the toolbar under the selection, or above it, or inside it as a last resort. */
function placeToolbar(r: Rect) {
  const tw = toolbar.offsetWidth;
  const th = toolbar.offsetHeight;
  const gap = 8;
  let top = r.y + r.h + gap;
  if (top + th > vh() - 4) top = r.y - th - gap;
  if (top < 4) top = r.y + r.h - th - gap;
  const left = clamp(r.x + r.w - tw, 4, vw() - tw - 4);
  toolbar.style.left = `${left}px`;
  const finalTop = clamp(top, 4, vh() - th - 4);
  toolbar.style.top = `${finalTop}px`;
  // Not enough room above the toolbar for tooltips: show them below it.
  toolbar.classList.toggle("tips-below", finalTop < 40);
}

function normalize(x1: number, y1: number, x2: number, y2: number): Rect {
  const x = clamp(Math.min(x1, x2), 0, vw());
  const y = clamp(Math.min(y1, y2), 0, vh());
  return {
    x,
    y,
    w: clamp(Math.max(x1, x2), 0, vw()) - x,
    h: clamp(Math.max(y1, y2), 0, vh()) - y,
  };
}

function resize(start: Rect, handle: Handle, px: number, py: number): Rect {
  let x1 = start.x;
  let y1 = start.y;
  let x2 = start.x + start.w;
  let y2 = start.y + start.h;
  if (handle.includes("w")) x1 = px;
  if (handle.includes("e")) x2 = px;
  if (handle.includes("n")) y1 = py;
  if (handle.includes("s")) y2 = py;
  return normalize(x1, y1, x2, y2);
}

function onPointerDown(e: PointerEvent) {
  if (busy || e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (toolbar.contains(target)) return;

  const handle = target.dataset.h as Handle | undefined;
  if (handle && sel) {
    drag = { kind: "resize", handle, start: { ...sel } };
  } else if (target === selEl && sel) {
    drag = { kind: "move", ox: e.clientX, oy: e.clientY, start: { ...sel } };
  } else {
    drag = { kind: "create", ox: e.clientX, oy: e.clientY };
    sel = { x: e.clientX, y: e.clientY, w: 0, h: 0 };
  }
  fullScreen = false;
  document.body.setPointerCapture(e.pointerId);
  render();
}

function onPointerMove(e: PointerEvent) {
  if (!drag) return;
  if (drag.kind === "create") {
    sel = normalize(drag.ox, drag.oy, e.clientX, e.clientY);
  } else if (drag.kind === "move") {
    const { start } = drag;
    sel = {
      ...start,
      x: clamp(start.x + e.clientX - drag.ox, 0, vw() - start.w),
      y: clamp(start.y + e.clientY - drag.oy, 0, vh() - start.h),
    };
  } else {
    sel = resize(drag.start, drag.handle, e.clientX, e.clientY);
  }
  render();
}

function onPointerUp(e: PointerEvent) {
  if (!drag) return;
  drag = null;
  if (document.body.hasPointerCapture(e.pointerId)) document.body.releasePointerCapture(e.pointerId);
  // A plain click (no drag) on empty space clears the selection.
  if (sel && (sel.w < MIN_SIZE || sel.h < MIN_SIZE)) sel = null;
  render();
}

function selectFullScreen() {
  fullScreen = true;
  sel = { x: 0, y: 0, w: vw(), h: vh() };
  render();
}

function showToast(text: string) {
  toast.textContent = text;
  toast.hidden = false;
}

async function close() {
  await invoke("close_capture");
}

async function run(action: string) {
  if (busy) return;
  if (action === "close") return close();
  if (action === "full") return selectFullScreen();
  if (!sel) return;

  busy = true;
  const rect = toImageRect(sel);
  try {
    if (action === "copy") {
      await invoke("copy_selection", { rect });
    } else if (action === "save") {
      const path = await invoke<string>("save_selection", { rect });
      toolbar.hidden = true;
      showToast(`Salvo em ${path}`);
      setTimeout(close, 900);
    } else if (action === "saveas") {
      await invoke("save_selection_as", { rect });
    }
  } catch (err) {
    showToast(`Erro: ${err}`);
    busy = false;
  }
}

function onKeyDown(e: KeyboardEvent) {
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  if (e.key === "Escape") {
    e.preventDefault();
    if (drag) {
      drag = null;
      sel = null;
      render();
    } else {
      void close();
    }
  } else if (e.key === "Enter" || (mod && key === "c")) {
    e.preventDefault();
    void run("copy");
  } else if (mod && key === "s") {
    e.preventDefault();
    void run(e.shiftKey ? "saveas" : "save");
  } else if (key === "f" || (mod && key === "a")) {
    e.preventDefault();
    selectFullScreen();
  }
}

async function init() {
  if (navigator.userAgent.includes("Mac")) {
    toolbar.querySelectorAll<HTMLElement>("[data-tip]").forEach((b) => {
      b.dataset.tip = b.dataset.tip!.replace(/Ctrl\+/g, "⌘");
    });
  }
  document.addEventListener("pointerdown", onPointerDown);
  document.addEventListener("pointermove", onPointerMove);
  document.addEventListener("pointerup", onPointerUp);
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    void close();
  });
  selEl.addEventListener("dblclick", () => void run("copy"));
  toolbar.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest("button");
    if (btn?.dataset.action) void run(btn.dataset.action);
  });
  // The window may get its final size after the page loads.
  window.addEventListener("resize", () => (fullScreen ? selectFullScreen() : render()));

  // The window starts hidden: show it only once the screenshot is decoded, so the
  // user never sees an empty frame.
  shot.src = convertFileSrc(`${monitor}/${nonce}`, "kadro");
  await shot.decode();
  if (startFull) selectFullScreen();
  render();
  const win = getCurrentWindow();
  await win.show();
  await win.setFocus();
}

init().catch((err) => {
  console.error(err);
  void close();
});
