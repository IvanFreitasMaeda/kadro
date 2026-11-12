// Annotation shapes and how to draw them on a 2D canvas. Shapes are stored in CSS
// pixels of the overlay window, so the same code draws them on screen and, with a
// scale transform, on the full-resolution image that gets copied or saved.

export type Tool = "pen" | "marker" | "line" | "arrow" | "rect" | "ellipse" | "text" | "blur";
export type Pt = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };

export type Shape =
  | { kind: "pen" | "marker"; color: string; width: number; pts: Pt[] }
  | { kind: "line" | "arrow" | "rect" | "ellipse"; color: string; width: number; a: Pt; b: Pt }
  | { kind: "text"; color: string; size: number; at: Pt; text: string }
  // `block` is the side of the pixel blocks, in CSS pixels.
  | { kind: "blur"; block: number; a: Pt; b: Pt };

export const COLORS = [
  "#ff3b30",
  "#ff9500",
  "#ffd60a",
  "#34c759",
  "#0a84ff",
  "#af52de",
  "#ffffff",
  "#1c1c1e",
];

/** Thickness presets: line width, highlighter width, font size and blur block size. */
export const SIZES = [
  { name: "Fina", stroke: 2, marker: 10, font: 16, blur: 6 },
  { name: "Média", stroke: 4, marker: 18, font: 22, blur: 10 },
  { name: "Grossa", stroke: 8, marker: 30, font: 32, blur: 16 },
];

export const MARKER_ALPHA = 0.4;
export const LINE_HEIGHT = 1.25;
export const FONT_FAMILY = '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
export const font = (size: number) => `600 ${size}px ${FONT_FAMILY}`;

/** Snaps `b` so the segment a→b is a multiple of 45°, keeping its length. */
export function snapAngle(a: Pt, b: Pt): Pt {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len = Math.hypot(dx, dy);
  const step = Math.PI / 4;
  const ang = Math.round(Math.atan2(dy, dx) / step) * step;
  return { x: a.x + Math.cos(ang) * len, y: a.y + Math.sin(ang) * len };
}

/** Moves `b` so the box a→b is a square. */
export function snapSquare(a: Pt, b: Pt): Pt {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const side = Math.max(Math.abs(dx), Math.abs(dy));
  return { x: a.x + side * Math.sign(dx || 1), y: a.y + side * Math.sign(dy || 1) };
}

/** True when the shape is too small to be anything but an accidental click. */
export function isEmpty(s: Shape): boolean {
  if (s.kind === "text") return s.text.trim() === "";
  if ("pts" in s) return s.pts.length === 0;
  return Math.abs(s.b.x - s.a.x) < 2 && Math.abs(s.b.y - s.a.y) < 2;
}

function freehand(ctx: CanvasRenderingContext2D, pts: Pt[]) {
  ctx.beginPath();
  ctx.moveTo(pts[0].x, pts[0].y);
  if (pts.length === 1) {
    // A click leaves a dot: the round cap draws it.
    ctx.lineTo(pts[0].x + 0.01, pts[0].y);
  } else {
    // Curves through the midpoints smooth out the jitter of the mouse.
    for (let i = 1; i < pts.length - 1; i++) {
      const mx = (pts[i].x + pts[i + 1].x) / 2;
      const my = (pts[i].y + pts[i + 1].y) / 2;
      ctx.quadraticCurveTo(pts[i].x, pts[i].y, mx, my);
    }
    const last = pts[pts.length - 1];
    ctx.lineTo(last.x, last.y);
  }
  ctx.stroke();
}

function arrow(ctx: CanvasRenderingContext2D, a: Pt, b: Pt, width: number) {
  const len = Math.hypot(b.x - a.x, b.y - a.y);
  const head = Math.min(Math.max(12, width * 4), len);
  const ang = Math.atan2(b.y - a.y, b.x - a.x);
  const spread = Math.PI / 7;
  // The shaft stops inside the head so its round cap doesn't poke out of the tip.
  const back = head * 0.7;
  ctx.beginPath();
  ctx.moveTo(a.x, a.y);
  ctx.lineTo(b.x - Math.cos(ang) * back, b.y - Math.sin(ang) * back);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(b.x, b.y);
  ctx.lineTo(b.x - head * Math.cos(ang - spread), b.y - head * Math.sin(ang - spread));
  ctx.lineTo(b.x - head * Math.cos(ang + spread), b.y - head * Math.sin(ang + spread));
  ctx.closePath();
  ctx.fill();
}

/** Normalized box of a two-point shape. */
export function boxOf(s: { a: Pt; b: Pt }): Rect {
  return {
    x: Math.min(s.a.x, s.b.x),
    y: Math.min(s.a.y, s.b.y),
    w: Math.abs(s.b.x - s.a.x),
    h: Math.abs(s.b.y - s.a.y),
  };
}

export function drawShape(ctx: CanvasRenderingContext2D, s: Shape) {
  // Blur needs the screenshot's pixels: the overlay draws it with `drawAll`'s callback.
  if (s.kind === "blur") return;
  ctx.save();
  ctx.strokeStyle = s.color;
  ctx.fillStyle = s.color;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  switch (s.kind) {
    case "pen":
    case "marker":
      ctx.lineWidth = s.width;
      // One path for the whole stroke, so overlaps of a highlighter don't darken.
      if (s.kind === "marker") ctx.globalAlpha = MARKER_ALPHA;
      freehand(ctx, s.pts);
      break;
    case "line":
      ctx.lineWidth = s.width;
      ctx.beginPath();
      ctx.moveTo(s.a.x, s.a.y);
      ctx.lineTo(s.b.x, s.b.y);
      ctx.stroke();
      break;
    case "arrow":
      ctx.lineWidth = s.width;
      arrow(ctx, s.a, s.b, s.width);
      break;
    case "rect":
      ctx.lineWidth = s.width;
      ctx.strokeRect(
        Math.min(s.a.x, s.b.x),
        Math.min(s.a.y, s.b.y),
        Math.abs(s.b.x - s.a.x),
        Math.abs(s.b.y - s.a.y),
      );
      break;
    case "ellipse":
      ctx.lineWidth = s.width;
      ctx.beginPath();
      ctx.ellipse(
        (s.a.x + s.b.x) / 2,
        (s.a.y + s.b.y) / 2,
        Math.abs(s.b.x - s.a.x) / 2,
        Math.abs(s.b.y - s.a.y) / 2,
        0,
        0,
        Math.PI * 2,
      );
      ctx.stroke();
      break;
    case "text": {
      ctx.font = font(s.size);
      ctx.textBaseline = "middle";
      const lh = s.size * LINE_HEIGHT;
      s.text.split("\n").forEach((line, i) => {
        ctx.fillText(line, s.at.x, s.at.y + lh * i + lh / 2);
      });
      break;
    }
  }
  ctx.restore();
}

type BlurShape = Extract<Shape, { kind: "blur" }>;

/**
 * Draws every shape, clipped to `clip` when given. Blurred areas go first, through
 * `blur`, so the other annotations stay sharp on top of them, as in the saved image.
 */
export function drawAll(
  ctx: CanvasRenderingContext2D,
  shapes: Shape[],
  clip?: Rect,
  blur?: (ctx: CanvasRenderingContext2D, s: BlurShape) => void,
) {
  ctx.save();
  if (clip) {
    ctx.beginPath();
    ctx.rect(clip.x, clip.y, clip.w, clip.h);
    ctx.clip();
  }
  if (blur) for (const s of shapes) if (s.kind === "blur") blur(ctx, s);
  for (const s of shapes) drawShape(ctx, s);
  ctx.restore();
}
