// Settings shared by the draw overlay (draw.ts) and the recording bar (record.ts).

export type DrawTool = "pen" | "marker" | "arrow" | "rect";
export const DRAW_TOOLS: { tool: DrawTool; key: string; name: string }[] = [
  { tool: "pen", key: "P", name: "Caneta" },
  { tool: "marker", key: "M", name: "Marca-texto" },
  { tool: "arrow", key: "A", name: "Seta" },
  { tool: "rect", key: "R", name: "Retângulo" },
];
/** Bright colors that stand out on most screens; keys 1 to 5. */
export const DRAW_COLORS = ["#ff3b30", "#ffd60a", "#34c759", "#0a84ff", "#ffffff"];

/** What the bar shows and the overlay draws with. */
export type DrawState = { tool: DrawTool; color: string; fade: boolean };

/** Sent by the bar to the overlay: a change, or an action. */
export type DrawSet = Partial<DrawState> & { clear?: boolean; undo?: boolean };

/** A key pressed while the bar has the focus, handled by the overlay. */
export type DrawKey = { key: string; mod: boolean; shift: boolean };
