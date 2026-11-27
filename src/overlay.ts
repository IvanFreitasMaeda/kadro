// Capture overlay: shows the frozen screenshot of one monitor and lets the user
// select an area, annotate it, then copy it or save it. One overlay window exists
// per monitor. In recording mode it picks the area and format to record instead.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  COLORS,
  LINE_HEIGHT,
  SIZES,
  boxOf,
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
const IS_MAC = navigator.userAgent.includes("Mac");
/** How the copy modifier reads in tooltips. */
const MOD = IS_MAC ? "⌘" : "Ctrl+";
// Injected by the Rust side when it creates the overlay window.
declare global {
  interface Window {
    __KADRO__?: {
      monitor: number;
      nonce: number;
      full: boolean;
      /** This overlay is on the monitor under the cursor, so it takes the focus. */
      focus?: boolean;
      record?: boolean;
      /** Opened by the "copy text" shortcut: the text of the area is copied right away. */
      ocr?: boolean;
      format?: Format;
    };
  }
}
type Format = "mp4" | "gif";
const {
  monitor,
  nonce,
  full: startFull,
  focus: startFocused = true,
  record: recordMode = false,
  ocr: quickText = false,
  format: startFormat = "mp4",
} = window.__KADRO__ ?? { monitor: 0, nonce: 0, full: false };
let format: Format = startFormat;

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
const ocrEl = $("ocr");

const TOOL_KEYS: Record<string, Tool> = {
  p: "pen",
  m: "marker",
  b: "blur",
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

// Text mode (OCR): the words found in the selection, in CSS pixels and reading
// order, and the range of them the user picked.
type Word = { text: string; line: number; x: number; y: number; w: number; h: number };
type OcrWord = { text: string; x: number; y: number; w: number; h: number };
let words: Word[] | null = null;
let wordEls: HTMLElement[] = [];
/** Picked words, as indexes into `words` (anchor where the drag began). */
let pick: { anchor: number; focus: number } | null = null;
let pickDrag: { start: Pt; moved: boolean } | null = null;
/** Bumped when the selection changes, to drop OCR results for an older one. */
let ocrToken = 0;
let toastTimer = 0;

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
  toolbar.hidden = !has || drag !== null || quickText;
  tools.hidden = toolbar.hidden || recordMode;
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
  if (words && sel && !handle && inside(sel, p)) {
    startPick(p);
    document.body.setPointerCapture(e.pointerId);
    return;
  }
  // Moving or redrawing the selection drops the text read from the old one.
  exitText();
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
    claimSelection();
  }
  fullScreen = false;
  document.body.setPointerCapture(e.pointerId);
  render();
}

function onPointerMove(e: PointerEvent) {
  shiftDown = e.shiftKey;
  if (pickDrag) {
    movePick({ x: e.clientX, y: e.clientY });
    return;
  }
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
  if (pickDrag) {
    endPick();
    return;
  }
  if (current) {
    finishShape();
    return;
  }
  if (!drag) return;
  drag = null;
  // A plain click (no drag) on empty space clears the selection.
  if (sel && (sel.w < MIN_SIZE || sel.h < MIN_SIZE)) sel = null;
  render();
  if (quickText && sel) void copyAreaText();
}

// ---- Text (OCR) ----

/** Headers that tell the Rust side which part of the screenshot to use. */
function requestHeaders(rect: ReturnType<typeof toImageRect>) {
  const headers: Record<string, string> = { "x-kadro-rect": JSON.stringify(rect) };
  const blurs = shapes.flatMap((s) =>
    s.kind === "blur" ? [{ ...toImageRect(boxOf(s)), block: blockPx(s) }] : [],
  );
  if (blurs.length) headers["x-kadro-blur"] = JSON.stringify(blurs);
  return headers;
}

/** Reads the text in the selection; null if the selection changed meanwhile. */
async function readText(): Promise<Word[] | null> {
  if (!sel) return null;
  const token = ++ocrToken;
  const rect = toImageRect(sel);
  const sx = shot.naturalWidth / vw();
  const sy = shot.naturalHeight / vh();
  showToast("Reconhecendo texto…");
  const lines = await invoke<{ words: OcrWord[] }[]>("recognize_text", new Uint8Array(), {
    headers: requestHeaders(rect),
  });
  if (token !== ocrToken) return null;
  hideToast();
  return lines.flatMap((l, line) =>
    l.words.map((w) => ({
      text: w.text,
      line,
      x: (rect.x + w.x) / sx,
      y: (rect.y + w.y) / sy,
      w: w.w / sx,
      h: w.h / sy,
    })),
  );
}

/** Joins words with spaces, and lines with line breaks. */
function joinWords(list: Word[]) {
  let out = "";
  list.forEach((w, i) => {
    if (i > 0) out += list[i - 1].line === w.line ? " " : "\n";
    out += w.text;
  });
  return out;
}

async function enterText() {
  if (!sel || busy || words) return;
  setTool(null);
  busy = true;
  try {
    const found = await readText();
    if (!found) return;
    if (found.length === 0) {
      showToast("Nenhum texto encontrado nessa área.", 1800);
      return;
    }
    words = found;
    pick = null;
    document.body.classList.add("text-mode");
    tools.querySelector('[data-action="ocr"]')!.classList.add("active");
    buildWords();
    render();
  } catch (err) {
    showToast(`Erro: ${err}`, 3000);
  } finally {
    busy = false;
  }
}

function exitText() {
  ocrToken++;
  if (!words) return;
  words = null;
  pick = null;
  pickDrag = null;
  wordEls = [];
  ocrEl.replaceChildren();
  ocrEl.hidden = true;
  document.body.classList.remove("text-mode");
  tools.querySelector('[data-action="ocr"]')!.classList.remove("active");
}

function buildWords() {
  if (!words || !sel) return;
  Object.assign(ocrEl.style, {
    left: `${sel.x}px`,
    top: `${sel.y}px`,
    width: `${sel.w}px`,
    height: `${sel.h}px`,
  });
  wordEls = words.map((w) => {
    const el = document.createElement("div");
    el.className = "word";
    Object.assign(el.style, {
      left: `${w.x - sel!.x}px`,
      top: `${w.y - sel!.y}px`,
      width: `${w.w}px`,
      height: `${w.h}px`,
    });
    return el;
  });
  ocrEl.replaceChildren(...wordEls);
  ocrEl.hidden = false;
  updatePick();
}

/** The picked words, in reading order. */
function pickedWords(): Word[] {
  if (!words || !pick) return [];
  const a = Math.min(pick.anchor, pick.focus);
  const b = Math.max(pick.anchor, pick.focus);
  return words.slice(a, b + 1);
}

function updatePick() {
  const a = pick ? Math.min(pick.anchor, pick.focus) : -1;
  const b = pick ? Math.max(pick.anchor, pick.focus) : -2;
  wordEls.forEach((el, i) => el.classList.toggle("selected", i >= a && i <= b));
  const btn = toolbar.querySelector<HTMLElement>('[data-action="copytext"]')!;
  const n = pickedWords().length;
  btn.querySelector("span")!.textContent = n ? "Copiar seleção" : "Copiar tudo";
  const tip = `${n ? "Copiar o texto selecionado" : "Copiar todo o texto"} (Enter ou ${MOD}C)`;
  btn.dataset.tip = tip;
  btn.setAttribute("aria-label", tip);
  // The label's width changed: keep the toolbar aligned with the selection.
  if (sel && !toolbar.hidden) placeToolbar(sel);
}

/** The word under `p`, or -1. A few pixels of slack make small words easy to hit. */
function wordAt(p: Pt, slack = 3) {
  if (!words) return -1;
  return words.findIndex(
    (w) => p.x >= w.x - slack && p.x <= w.x + w.w + slack && p.y >= w.y - slack && p.y <= w.y + w.h + slack,
  );
}

/** The word closest to `p`, preferring words on the same line height. */
function nearestWord(p: Pt) {
  if (!words) return -1;
  let best = -1;
  let bestD = Infinity;
  words.forEach((w, i) => {
    const dx = Math.max(w.x - p.x, 0, p.x - (w.x + w.w));
    const dy = Math.max(w.y - p.y, 0, p.y - (w.y + w.h));
    const d = dx + dy * 4;
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  });
  return best;
}

function startPick(p: Pt) {
  // A click on a word picks it; on empty space it clears the pick, unless it
  // turns into a drag.
  const hit = wordAt(p);
  pickDrag = { start: p, moved: false };
  pick = hit >= 0 ? { anchor: hit, focus: hit } : null;
  updatePick();
}

function movePick(p: Pt) {
  if (!pickDrag || !words) return;
  if (!pickDrag.moved && Math.hypot(p.x - pickDrag.start.x, p.y - pickDrag.start.y) < 3) return;
  pickDrag.moved = true;
  const focus = nearestWord(p);
  if (focus < 0) return;
  if (!pick) pick = { anchor: nearestWord(pickDrag.start), focus };
  else pick.focus = focus;
  updatePick();
}

function endPick() {
  pickDrag = null;
}

/** Picks the whole line under `p` (double click). */
function pickLine(p: Pt) {
  if (!words) return;
  const i = wordAt(p);
  if (i < 0) return;
  const line = words[i].line;
  const first = words.findIndex((w) => w.line === line);
  let last = first;
  while (last + 1 < words.length && words[last + 1].line === line) last++;
  pick = { anchor: first, focus: last };
  updatePick();
}

function pickAll() {
  if (!words?.length) return;
  pick = { anchor: 0, focus: words.length - 1 };
  updatePick();
}

/** Copies the picked words, or all of them, and closes the overlay. */
async function copyText() {
  if (!words || busy) return;
  const picked = pickedWords();
  const text = joinWords(picked.length ? picked : words);
  busy = true;
  try {
    await invoke("copy_text", { text, close: true });
  } catch (err) {
    showToast(`Erro: ${err}`, 3000);
    busy = false;
  }
}

/** "Copy text" shortcut: reads the area just picked and copies all of its text. */
async function copyAreaText() {
  if (busy) return;
  busy = true;
  try {
    const found = await readText();
    if (!found) return;
    if (found.length === 0) {
      sel = null;
      render();
      showToast("Nenhum texto encontrado. Arraste sobre outra área.", 2200);
      return;
    }
    await invoke("copy_text", { text: joinWords(found), close: true });
  } catch (err) {
    showToast(`Erro: ${err}`, 3000);
  } finally {
    busy = false;
  }
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
  drawAll(inkCtx, current ? [...shapes, current] : shapes, sel, drawBlur);
}

// Blur: the area is pixelated, which hides text for good, unlike a soft blur that
// can sometimes be undone. The preview draws a scaled-down copy of the screenshot
// back up without smoothing; the Rust side does the same with exact averages when
// the image is copied or saved, on the same grid of blocks.

/** Side of a blur shape's blocks in screenshot pixels. */
const blockPx = (s: { block: number }) => Math.max(2, Math.round((s.block * shot.naturalWidth) / vw()));

/** The screenshot shrunk so each pixel stands for one block, per block size. */
const mosaics = new Map<number, HTMLCanvasElement>();

function mosaic(block: number) {
  let c = mosaics.get(block);
  if (!c) {
    c = document.createElement("canvas");
    c.width = Math.ceil(shot.naturalWidth / block);
    c.height = Math.ceil(shot.naturalHeight / block);
    const cx = c.getContext("2d")!;
    cx.imageSmoothingQuality = "high";
    cx.drawImage(shot, 0, 0, shot.naturalWidth / block, shot.naturalHeight / block);
    mosaics.set(block, c);
  }
  return c;
}

function drawBlur(ctx: CanvasRenderingContext2D, s: Extract<Shape, { kind: "blur" }>) {
  const r = boxOf(s);
  const block = blockPx(s);
  const m = mosaic(block);
  // Screenshot pixels to CSS pixels.
  const kx = vw() / shot.naturalWidth;
  const ky = vh() / shot.naturalHeight;
  const i0 = clamp(Math.floor(r.x / kx / block), 0, m.width);
  const j0 = clamp(Math.floor(r.y / ky / block), 0, m.height);
  const i1 = clamp(Math.ceil((r.x + r.w) / kx / block), 0, m.width);
  const j1 = clamp(Math.ceil((r.y + r.h) / ky / block), 0, m.height);
  if (i1 <= i0 || j1 <= j0) return;
  ctx.save();
  ctx.beginPath();
  ctx.rect(r.x, r.y, r.w, r.h);
  ctx.clip();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(
    m,
    i0,
    j0,
    i1 - i0,
    j1 - j0,
    i0 * block * kx,
    j0 * block * ky,
    (i1 - i0) * block * kx,
    (j1 - j0) * block * ky,
  );
  ctx.restore();
}

function startShape(p: Pt) {
  const size = SIZES[sizeIdx];
  if (tool === "blur") {
    current = { kind: "blur", block: size.blur, a: p, b: p };
  } else if (tool === "pen" || tool === "marker") {
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
    else if (current.kind === "rect" || current.kind === "ellipse" || current.kind === "blur")
      current.b = snapSquare(current.a, p);
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
  // Blurred areas are applied by the Rust side, straight on the screenshot.
  if (shapes.every((s) => s.kind === "blur")) return new Uint8Array();
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
  claimSelection();
  exitText();
  fullScreen = true;
  sel = { x: 0, y: 0, w: vw(), h: vh() };
  render();
}

// ---- One selection across monitors ----
// Copy and save use the selection of the overlay they are pressed on and then close
// every overlay, so a selection left on another monitor would be lost silently.
// Starting a selection on one monitor clears the others, like a single screen.

const CLAIM_EVENT = "kadro://selection-claimed";

function claimSelection() {
  void emit(CLAIM_EVENT, { monitor, nonce });
}

/** Drops this overlay's selection and annotations because another monitor took over. */
function clearSelection() {
  cancelText();
  exitText();
  sel = null;
  drag = null;
  current = null;
  fullScreen = false;
  shapes = [];
  redoStack = [];
  updateHistoryButtons();
  hideToast();
  render();
}

/** Shows a message at the bottom; `ms` hides it after that long. */
function showToast(text: string, ms = 0) {
  clearTimeout(toastTimer);
  toast.textContent = text;
  toast.hidden = false;
  if (ms) toastTimer = window.setTimeout(hideToast, ms);
}

function hideToast() {
  clearTimeout(toastTimer);
  toast.hidden = true;
}

async function close() {
  await invoke("close_capture");
}

function setFormat(f: Format) {
  format = f;
  toolbar.querySelectorAll<HTMLElement>("[data-format]").forEach((b) => {
    b.classList.toggle("active", b.dataset.format === f);
    b.setAttribute("aria-checked", String(b.dataset.format === f));
  });
}

async function startRecording() {
  if (!sel) return;
  busy = true;
  try {
    await invoke("start_recording", { rect: toImageRect(sel), format });
  } catch (err) {
    showToast(`Erro: ${err}`);
    busy = false;
  }
}

async function run(action: string) {
  if (busy) return;
  if (action === "close") return close();
  if (action === "full") {
    selectFullScreen();
    if (quickText) void copyAreaText();
    return;
  }
  if (!sel) return;
  if (action === "copytext") return copyText();
  if (recordMode) {
    if (action === "record") await startRecording();
    return;
  }

  busy = true;
  commitText();
  closePopovers();
  const rect = toImageRect(sel);
  try {
    // The annotations travel as a PNG in the request body and the selection in a
    // header; the Rust side crops the screenshot and lays the annotations on top.
    const body = await inkPng(rect);
    const options = { headers: requestHeaders(rect) };
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
  if (btn.dataset.action === "ocr") {
    closePopovers();
    if (words) exitText();
    else void enterText();
  } else if (btn.dataset.tool) {
    const t = btn.dataset.tool as Tool;
    closePopovers();
    exitText();
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
  if (words) {
    // Text mode keys: pick and copy words; anything else falls through.
    if (e.key === "Escape") {
      e.preventDefault();
      if (pick) {
        pick = null;
        updatePick();
      } else exitText();
      return;
    }
    if (e.key === "Enter" || (mod && key === "c")) {
      e.preventDefault();
      void copyText();
      return;
    }
    if (mod && key === "a") {
      e.preventDefault();
      pickAll();
      return;
    }
    if (!mod && key === "o") {
      e.preventDefault();
      exitText();
      return;
    }
  }
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
  } else if (recordMode) {
    if (e.key === "Enter") {
      e.preventDefault();
      void run("record");
    } else if (key === "f" || (mod && key === "a")) {
      e.preventDefault();
      selectFullScreen();
    } else if (!mod && (key === "v" || key === "g")) {
      e.preventDefault();
      setFormat(key === "g" ? "gif" : "mp4");
    }
  } else if (quickText) {
    if (key === "f" || (mod && key === "a")) {
      e.preventDefault();
      void run("full");
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
  } else if (!mod && !e.altKey && key === "o" && sel) {
    e.preventDefault();
    void enterText();
  } else if (!mod && !e.altKey && TOOL_KEYS[key] && sel) {
    e.preventDefault();
    const t = TOOL_KEYS[key];
    exitText();
    setTool(tool === t ? null : t);
  }
}

async function init() {
  if (recordMode) {
    document.body.classList.add("record");
    hint.innerHTML =
      "Arraste para escolher o que gravar · <kbd>F</kbd> tela inteira · <kbd>Esc</kbd> cancelar";
    setFormat(format);
  } else if (quickText) {
    document.body.classList.add("quick-text");
    hint.innerHTML =
      "Arraste sobre o texto que quer copiar · <kbd>F</kbd> tela inteira · <kbd>Esc</kbd> cancelar";
  }
  buildPopovers();
  setColor(color);
  setSize(sizeIdx);
  updateHistoryButtons();
  if (IS_MAC) {
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
  selEl.addEventListener("dblclick", (e) => {
    if (words) pickLine({ x: e.clientX, y: e.clientY });
    else if (!tool && !quickText) void run(recordMode ? "record" : "copy");
  });
  // Buttons must not take focus away from the text box.
  for (const bar of [toolbar, tools]) bar.addEventListener("mousedown", (e) => e.preventDefault());
  toolbar.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest("button");
    if (btn?.dataset.format) setFormat(btn.dataset.format as Format);
    else if (btn?.dataset.action) void run(btn.dataset.action);
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
  await listen<{ monitor: number; nonce: number }>(CLAIM_EVENT, ({ payload }) => {
    if (payload.nonce === nonce && payload.monitor !== monitor && !busy) clearSelection();
  });
  const win = getCurrentWindow();
  await win.show();
  if (startFocused) await win.setFocus();
  // With several monitors, the keyboard (Esc, Enter, tool keys) follows the mouse.
  document.addEventListener("pointermove", () => {
    if (!document.hasFocus()) void win.setFocus();
  });
}

init().catch((err) => {
  console.error(err);
  void close();
});
