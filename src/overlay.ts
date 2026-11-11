// Capture overlay: shows the frozen screenshot of one monitor and lets the user
// select an area, annotate it, then copy it or save it. One overlay window exists
// per monitor.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  COLORS,
  LINE_HEIGHT,
  SIZES,
  drawAll,
  drawShape,
  font,
  isEmpty,
  snapAngle,
  snapSquare,
  type Pt,
  type Rect,
  type Shape,
  type Tool,
} from "./annotate";

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
const tools = $("tools");
const palette = $("palette");
const sizesEl = $("sizes");
const ink = $<HTMLCanvasElement>("ink");
const inkCtx = ink.getContext("2d")!;
const textEdit = $<HTMLTextAreaElement>("text-edit");

const TOOL_KEYS: Record<string, Tool> = {
  p: "pen",
  m: "marker",
  l: "line",
  a: "arrow",
  r: "rect",
  e: "ellipse",
  t: "text",
};

let sel: Rect | null = null;
let drag: Drag | null = null;
let busy = false;
/** True while the selection is the whole screen, so it follows window resizes. */
let fullScreen = false;

// Annotations
let tool: Tool | null = null;
let color = COLORS[0];
let sizeIdx = 1;
let shapes: Shape[] = [];
let redoStack: Shape[] = [];
/** The shape being drawn right now, not yet in `shapes`. */
let current: Shape | null = null;
/** Where the text being typed starts, while the text box is open. */
let textAt: Pt | null = null;
let shiftDown = false;

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
  tools.hidden = toolbar.hidden;
  if (tools.hidden) closePopovers();
  drawInk();
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

  if (!toolbar.hidden) {
    placeToolbar(sel);
    placeTools(sel);
  }
}

/** Puts the tools bar right of the selection, or left of it, or inside it. */
function placeTools(r: Rect) {
  const tw = tools.offsetWidth;
  const th = tools.offsetHeight;
  const gap = 8;
  let left = r.x + r.w + gap;
  let onLeft = false;
  if (left + tw > vw() - 4) {
    left = r.x - tw - gap;
    onLeft = true;
  }
  if (left < 4) {
    left = r.x + r.w - tw - gap;
    onLeft = true;
  }
  tools.style.left = `${clamp(left, 4, vw() - tw - 4)}px`;
  tools.style.top = `${clamp(r.y, 4, vh() - th - 4)}px`;
  tools.classList.toggle("on-left", onLeft);
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

const inside = (r: Rect, p: Pt) => p.x >= r.x && p.y >= r.y && p.x <= r.x + r.w && p.y <= r.y + r.h;

function onPointerDown(e: PointerEvent) {
  if (busy || e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (toolbar.contains(target) || tools.contains(target) || target === textEdit) return;
  closePopovers();
  // Clicking anywhere else finishes the text being typed.
  const wasTyping = textAt !== null;
  if (wasTyping) commitText();

  const p = { x: e.clientX, y: e.clientY };
  const handle = target.dataset.h as Handle | undefined;
  if (handle && sel) {
    drag = { kind: "resize", handle, start: { ...sel } };
  } else if (tool && sel) {
    // With a tool picked, the selection is a canvas: drawing outside it is ignored.
    if (!inside(sel, p)) return;
    if (tool === "text") {
      // Keep this click from moving focus, so the text box gets every keystroke.
      e.preventDefault();
      if (!wasTyping) openText(p);
      return;
    }
    startShape(p);
    document.body.setPointerCapture(e.pointerId);
    return;
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
  shiftDown = e.shiftKey;
  if (current) {
    updateShape({ x: e.clientX, y: e.clientY });
    return;
  }
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
  if (document.body.hasPointerCapture(e.pointerId)) document.body.releasePointerCapture(e.pointerId);
  if (current) {
    finishShape();
    return;
  }
  if (!drag) return;
  drag = null;
  // A plain click (no drag) on empty space clears the selection.
  if (sel && (sel.w < MIN_SIZE || sel.h < MIN_SIZE)) sel = null;
  render();
}

// ---- Annotations ----

/** Redraws the annotations, clipped to the selection like the final image. */
function drawInk() {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(vw() * dpr);
  const h = Math.round(vh() * dpr);
  if (ink.width !== w || ink.height !== h) {
    ink.width = w;
    ink.height = h;
  }
  inkCtx.setTransform(1, 0, 0, 1, 0, 0);
  inkCtx.clearRect(0, 0, w, h);
  if (!sel || sel.w <= 0 || sel.h <= 0) return;
  inkCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
  drawAll(inkCtx, current ? [...shapes, current] : shapes, sel);
}

function startShape(p: Pt) {
  const size = SIZES[sizeIdx];
  if (tool === "pen" || tool === "marker") {
    current = {
      kind: tool,
      color,
      width: tool === "pen" ? size.stroke : size.marker,
      pts: [p],
    };
  } else if (tool && tool !== "text") {
    current = { kind: tool, color, width: size.stroke, a: p, b: p };
  }
  drawInk();
}

function updateShape(p: Pt) {
  if (!current) return;
  if ("pts" in current) {
    const last = current.pts[current.pts.length - 1];
    if (Math.abs(last.x - p.x) + Math.abs(last.y - p.y) < 1) return;
    current.pts.push(p);
  } else if ("b" in current) {
    if (!shiftDown) current.b = p;
    else if (current.kind === "rect" || current.kind === "ellipse") current.b = snapSquare(current.a, p);
    else current.b = snapAngle(current.a, p);
  }
  drawInk();
}

function finishShape() {
  if (current && !isEmpty(current)) addShape(current);
  current = null;
  drawInk();
}

function addShape(s: Shape) {
  shapes.push(s);
  redoStack = [];
  updateHistoryButtons();
}

function undo() {
  if (textAt) return cancelText();
  const s = shapes.pop();
  if (s) redoStack.push(s);
  updateHistoryButtons();
  drawInk();
}

function redo() {
  const s = redoStack.pop();
  if (s) shapes.push(s);
  updateHistoryButtons();
  drawInk();
}

function updateHistoryButtons() {
  tools.querySelector<HTMLButtonElement>('[data-action="undo"]')!.disabled = shapes.length === 0;
  tools.querySelector<HTMLButtonElement>('[data-action="redo"]')!.disabled = redoStack.length === 0;
}

function setTool(t: Tool | null) {
  if (textAt) commitText();
  tool = t;
  tools.querySelectorAll<HTMLElement>("[data-tool]").forEach((b) => {
    b.classList.toggle("active", b.dataset.tool === t);
  });
  document.body.classList.toggle("drawing", t !== null && t !== "text");
  document.body.classList.toggle("typing", t === "text");
}

function setColor(c: string) {
  color = c;
  document.documentElement.style.setProperty("--ink", c);
  palette.querySelectorAll<HTMLElement>("button").forEach((b) => {
    b.classList.toggle("active", b.dataset.color === c);
  });
  if (textAt) styleTextEdit();
}

function setSize(i: number) {
  sizeIdx = i;
  const size = SIZES[i];
  tools.querySelector(".dot")!.setAttribute("r", String(2 + i * 2));
  const btn = tools.querySelector<HTMLElement>('[data-action="size"]')!;
  btn.dataset.tip = `Espessura: ${size.name}`;
  btn.setAttribute("aria-label", btn.dataset.tip);
  sizesEl.querySelectorAll<HTMLElement>("button").forEach((b) => {
    b.classList.toggle("active", b.dataset.size === String(i));
  });
  if (textAt) styleTextEdit();
}

function togglePopover(el: HTMLElement, anchor: HTMLElement) {
  const open = el.hidden;
  closePopovers();
  if (!open) return;
  el.hidden = false;
  el.style.top = `${anchor.offsetTop}px`;
  tools.classList.add("popover-open");
}

function closePopovers() {
  palette.hidden = true;
  sizesEl.hidden = true;
  tools.classList.remove("popover-open");
}

function buildPopovers() {
  for (const c of COLORS) {
    const b = document.createElement("button");
    b.dataset.color = c;
    b.setAttribute("aria-label", c);
    b.innerHTML = `<span class="swatch" style="--c:${c}"></span>`;
    palette.append(b);
  }
  SIZES.forEach((size, i) => {
    const b = document.createElement("button");
    b.dataset.size = String(i);
    b.innerHTML = `<svg viewBox="0 0 24 24"><circle class="dot" cx="12" cy="12" r="${2 + i * 2}" /></svg>${size.name}`;
    sizesEl.append(b);
  });
}

// Text: a transparent text box sits where the text will be drawn, styled the same
// way, and becomes a shape when the user clicks elsewhere or presses Enter.

function styleTextEdit() {
  const px = SIZES[sizeIdx].font;
  Object.assign(textEdit.style, {
    font: font(px),
    lineHeight: `${px * LINE_HEIGHT}px`,
    color,
  });
  autosizeText();
}

function autosizeText() {
  textEdit.style.width = "0";
  textEdit.style.height = "0";
  textEdit.style.width = `${textEdit.scrollWidth + 2}px`;
  textEdit.style.height = `${textEdit.scrollHeight}px`;
}

function openText(p: Pt) {
  const lh = SIZES[sizeIdx].font * LINE_HEIGHT;
  textAt = { x: p.x, y: p.y - lh / 2 };
  textEdit.value = "";
  textEdit.style.left = `${textAt.x}px`;
  textEdit.style.top = `${textAt.y}px`;
  textEdit.hidden = false;
  styleTextEdit();
  textEdit.focus();
}

function commitText() {
  if (!textAt) return;
  const shape: Shape = {
    kind: "text",
    color,
    size: SIZES[sizeIdx].font,
    at: textAt,
    text: textEdit.value.replace(/\s+$/, ""),
  };
  cancelText();
  if (!isEmpty(shape)) addShape(shape);
  drawInk();
}

function cancelText() {
  textAt = null;
  textEdit.hidden = true;
  textEdit.value = "";
  textEdit.blur();
}

/** Renders the annotations at the screenshot's resolution, cropped to `img`. */
async function inkPng(img: { x: number; y: number; width: number; height: number }) {
  if (shapes.length === 0) return new Uint8Array();
  const canvas = document.createElement("canvas");
  canvas.width = img.width;
  canvas.height = img.height;
  const ctx = canvas.getContext("2d")!;
  const sx = shot.naturalWidth / vw();
  const sy = shot.naturalHeight / vh();
  ctx.setTransform(sx, 0, 0, sy, -img.x, -img.y);
  for (const s of shapes) drawShape(ctx, s);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("não foi possível desenhar as anotações");
  return new Uint8Array(await blob.arrayBuffer());
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
  commitText();
  closePopovers();
  const rect = toImageRect(sel);
  try {
    // The annotations travel as a PNG in the request body and the selection in a
    // header; the Rust side crops the screenshot and lays the annotations on top.
    const body = await inkPng(rect);
    const options = { headers: { "x-kadro-rect": JSON.stringify(rect) } };
    if (action === "copy") {
      await invoke("copy_selection", body, options);
    } else if (action === "save") {
      const path = await invoke<string>("save_selection", body, options);
      toolbar.hidden = true;
      tools.hidden = true;
      showToast(`Salvo em ${path}`);
      setTimeout(close, 900);
    } else if (action === "saveas") {
      await invoke("save_selection_as", body, options);
    }
  } catch (err) {
    showToast(`Erro: ${err}`);
    busy = false;
  }
}

function onToolsClick(e: MouseEvent) {
  const btn = (e.target as HTMLElement).closest("button");
  if (!btn) return;
  if (btn.dataset.tool) {
    const t = btn.dataset.tool as Tool;
    closePopovers();
    setTool(tool === t ? null : t);
  } else if (btn.dataset.color) {
    setColor(btn.dataset.color);
    closePopovers();
  } else if (btn.dataset.size) {
    setSize(Number(btn.dataset.size));
    closePopovers();
  } else if (btn.dataset.action === "color") {
    togglePopover(palette, btn);
  } else if (btn.dataset.action === "size") {
    togglePopover(sizesEl, btn);
  } else if (btn.dataset.action === "undo") {
    undo();
  } else if (btn.dataset.action === "redo") {
    redo();
  }
  // Keep typing in the text box after picking a color or size.
  if (textAt) textEdit.focus();
}

function onTextKeyDown(e: KeyboardEvent) {
  // Typing must not trigger the overlay's shortcuts.
  e.stopPropagation();
  if (e.key === "Escape") {
    e.preventDefault();
    cancelText();
    drawInk();
  } else if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    commitText();
  }
}

function onKeyDown(e: KeyboardEvent) {
  shiftDown = e.shiftKey;
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  if (e.key === "Escape") {
    e.preventDefault();
    if (current) {
      current = null;
      drawInk();
    } else if (!palette.hidden || !sizesEl.hidden) {
      closePopovers();
    } else if (drag) {
      drag = null;
      sel = null;
      render();
    } else {
      void close();
    }
  } else if (mod && key === "z") {
    e.preventDefault();
    if (e.shiftKey) redo();
    else undo();
  } else if (mod && key === "y") {
    e.preventDefault();
    redo();
  } else if (e.key === "Enter" || (mod && key === "c")) {
    e.preventDefault();
    void run("copy");
  } else if (mod && key === "s") {
    e.preventDefault();
    void run(e.shiftKey ? "saveas" : "save");
  } else if (key === "f" || (mod && key === "a")) {
    e.preventDefault();
    selectFullScreen();
  } else if (!mod && !e.altKey && TOOL_KEYS[key] && sel) {
    e.preventDefault();
    const t = TOOL_KEYS[key];
    setTool(tool === t ? null : t);
  }
}

async function init() {
  buildPopovers();
  setColor(color);
  setSize(sizeIdx);
  updateHistoryButtons();
  if (navigator.userAgent.includes("Mac")) {
    document.querySelectorAll<HTMLElement>("[data-tip]").forEach((b) => {
      b.dataset.tip = b.dataset.tip!.replace(/Ctrl\+/g, "⌘");
      b.setAttribute("aria-label", b.dataset.tip);
    });
  }
  document.addEventListener("pointerdown", onPointerDown);
  document.addEventListener("pointermove", onPointerMove);
  document.addEventListener("pointerup", onPointerUp);
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("keyup", (e) => (shiftDown = e.shiftKey));
  document.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    void close();
  });
  selEl.addEventListener("dblclick", () => {
    if (!tool) void run("copy");
  });
  // Buttons must not take focus away from the text box.
  for (const bar of [toolbar, tools]) bar.addEventListener("mousedown", (e) => e.preventDefault());
  toolbar.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest("button");
    if (btn?.dataset.action) void run(btn.dataset.action);
  });
  tools.addEventListener("click", onToolsClick);
  textEdit.addEventListener("keydown", onTextKeyDown);
  textEdit.addEventListener("input", autosizeText);
  textEdit.addEventListener("blur", () => {
    // Losing focus to anything but the tools bar finishes the text.
    setTimeout(() => {
      if (textAt && document.activeElement !== textEdit) commitText();
    }, 0);
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
