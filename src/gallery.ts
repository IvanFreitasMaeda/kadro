// Side gallery: recent captures as thumbnails. Click one to copy it; the buttons
// on each thumbnail save, open or remove it.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { pretty } from "./keys";

type Item = { id: string; time: number; width: number; height: number };
type Op = "copy" | "save" | "saveas" | "open" | "delete";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const list = $("list");
const empty = $("empty");
const count = $("count");
const status = $("status");
const tip = $("tip");
const clearBtn = $("clear");
const template = $<HTMLTemplateElement>("card");

const isMac = navigator.userAgent.includes("Mac");
const DEFAULT_STATUS = "Clique numa captura para copiar";

let items: Item[] = [];
let selected = -1;
/** True while a save dialog is open, so losing focus to it doesn't close the panel. */
let dialogOpen = false;
let statusTimer = 0;

function showStatus(text: string, kind: "ok" | "error" | "" = "") {
  status.textContent = text;
  status.className = kind;
  status.title = text;
  clearTimeout(statusTimer);
  if (kind) statusTimer = window.setTimeout(() => showStatus(DEFAULT_STATUS), 4000);
}

const timeFmt = new Intl.DateTimeFormat("pt-BR", { hour: "2-digit", minute: "2-digit" });
const dateFmt = new Intl.DateTimeFormat("pt-BR", { day: "2-digit", month: "2-digit" });

/** "agora", "há 5 min", "hoje 14:32", "ontem 09:10" or "03/11 18:45". */
function when(time: number): string {
  const minutes = Math.floor((Date.now() - time) / 60000);
  if (minutes < 1) return "agora";
  if (minutes < 60) return `há ${minutes} min`;
  const d = new Date(time);
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return `hoje ${timeFmt.format(d)}`;
  if (d.toDateString() === yesterday.toDateString()) return `ontem ${timeFmt.format(d)}`;
  return `${dateFmt.format(d)} ${timeFmt.format(d)}`;
}

function cards(): HTMLElement[] {
  return [...list.children] as HTMLElement[];
}

function render() {
  const scroll = list.scrollTop;
  list.replaceChildren(
    ...items.map((item) => {
      const card = template.content.firstElementChild!.cloneNode(true) as HTMLElement;
      card.dataset.id = item.id;
      const img = card.querySelector("img")!;
      img.src = convertFileSrc(`h/${item.id}.jpg`, "kadro");
      card.querySelector(".time")!.textContent = when(item.time);
      (card.querySelector(".time") as HTMLElement).title = new Date(item.time).toLocaleString("pt-BR");
      card.querySelector(".dims")!.textContent = `${item.width} × ${item.height}`;
      return card;
    }),
  );
  list.scrollTop = scroll;
  empty.hidden = items.length > 0;
  list.hidden = items.length === 0;
  clearBtn.hidden = items.length === 0;
  count.textContent = items.length ? String(items.length) : "";
  select(Math.min(Math.max(selected, 0), items.length - 1), false);
}

function select(index: number, scroll = true) {
  selected = index;
  cards().forEach((card, i) => card.classList.toggle("selected", i === index));
  if (scroll && index >= 0) cards()[index].scrollIntoView({ block: "nearest" });
}

async function load() {
  items = await invoke<Item[]>("history_list");
  render();
}

async function run(op: Op, index: number) {
  const item = items[index];
  if (!item) return;
  select(index);
  const { id } = item;
  try {
    if (op === "copy") {
      await invoke("history_copy", { id });
      const flash = cards()[index]?.querySelector(".flash");
      flash?.classList.remove("show");
      void (flash as HTMLElement | undefined)?.offsetWidth; // restart the animation
      flash?.classList.add("show");
      showStatus("Copiada para a área de transferência", "ok");
    } else if (op === "save") {
      const path = await invoke<string>("history_save", { id });
      showStatus(`Salva em ${path}`, "ok");
    } else if (op === "saveas") {
      dialogOpen = true;
      try {
        const path = await invoke<string | null>("history_save_as", { id });
        if (path) showStatus(`Salva em ${path}`, "ok");
      } finally {
        dialogOpen = false;
        await getCurrentWindow().setFocus();
      }
    } else if (op === "open") {
      await invoke("history_open", { id });
    } else if (op === "delete") {
      await invoke("history_delete", { id });
      items = items.filter((i) => i.id !== id);
      render();
      showStatus("Removida do histórico", "ok");
    }
  } catch (err) {
    showStatus(String(err), "error");
    // The capture may have been removed meanwhile (history limit lowered).
    void load();
  }
}

let clearTimer = 0;
async function clearAll() {
  // Two clicks: the first one only arms the button.
  if (!clearBtn.classList.contains("confirm")) {
    clearBtn.classList.add("confirm");
    clearBtn.dataset.tip = "Clique de novo para apagar tudo";
    showTip(clearBtn);
    clearTimer = window.setTimeout(disarmClear, 3000);
    return;
  }
  disarmClear();
  hideTip();
  try {
    await invoke("history_clear");
    items = [];
    render();
    showStatus("Histórico apagado", "ok");
  } catch (err) {
    showStatus(String(err), "error");
  }
}

function disarmClear() {
  clearTimeout(clearTimer);
  clearBtn.classList.remove("confirm");
  clearBtn.dataset.tip = "Apagar todo o histórico";
}

async function close() {
  await invoke("close_gallery");
}

// Tooltips: one element, kept inside the window.
let tipTimer = 0;
function showTip(target: HTMLElement) {
  tip.textContent = target.dataset.tip ?? "";
  tip.hidden = false;
  const r = target.getBoundingClientRect();
  const t = tip.getBoundingClientRect();
  const x = Math.min(Math.max(8, r.left + r.width / 2 - t.width / 2), window.innerWidth - t.width - 8);
  const below = r.top - t.height - 8 < 0;
  tip.style.left = `${x}px`;
  tip.style.top = `${below ? r.bottom + 6 : r.top - t.height - 6}px`;
}
function hideTip() {
  clearTimeout(tipTimer);
  tip.hidden = true;
}

function onKeyDown(e: KeyboardEvent) {
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  if (e.key === "Escape") {
    e.preventDefault();
    void close();
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!items.length) return;
    const step = e.key === "ArrowDown" ? 1 : -1;
    select(Math.min(Math.max(selected + step, 0), items.length - 1));
  } else if (selected < 0) {
    return;
  } else if (e.key === "Enter" || (mod && key === "c")) {
    e.preventDefault();
    void run("copy", selected);
  } else if (mod && key === "s") {
    e.preventDefault();
    void run(e.shiftKey ? "saveas" : "save", selected);
  } else if (e.key === " ") {
    e.preventDefault();
    void run("open", selected);
  } else if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    void run("delete", selected);
  }
}

async function init() {
  if (isMac) {
    document.querySelectorAll<HTMLElement>("[data-tip]").forEach((b) => {
      b.dataset.tip = b.dataset.tip!.replace(/Ctrl\+Shift\+/g, "⇧⌘").replace(/Ctrl\+/g, "⌘");
    });
  }
  showStatus(DEFAULT_STATUS);

  list.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const card = target.closest<HTMLElement>(".card");
    if (!card) return;
    const index = cards().indexOf(card);
    const op = target.closest<HTMLElement>("[data-op]")?.dataset.op as Op | undefined;
    if (op) void run(op, index);
    else if (target.closest(".thumb")) void run("copy", index);
    else select(index);
  });
  // Buttons must not take the keyboard focus, or Enter and Space would also click them.
  document.addEventListener("mousedown", (e) => {
    if ((e.target as HTMLElement).closest("button")) e.preventDefault();
  });
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("mouseover", (e) => {
    const target = (e.target as HTMLElement).closest<HTMLElement>("[data-tip]");
    hideTip();
    if (target) tipTimer = window.setTimeout(() => showTip(target), 350);
  });
  list.addEventListener("scroll", hideTip, { passive: true });
  clearBtn.addEventListener("click", () => void clearAll());
  $("close").addEventListener("click", () => void close());

  await listen("history-changed", () => void load());
  // Keep "há 5 min" labels current while the panel is open.
  setInterval(() => {
    cards().forEach((card, i) => {
      if (items[i]) card.querySelector(".time")!.textContent = when(items[i].time);
    });
  }, 30000);

  const [settings] = await Promise.all([
    invoke<{ shortcut_area: string }>("get_settings"),
    load(),
  ]);
  $("shortcut-area").textContent = pretty(settings.shortcut_area, isMac);

  // Like a menu: the panel closes when you click anywhere else.
  const win = getCurrentWindow();
  let focusedOnce = false;
  await win.onFocusChanged(({ payload: focused }) => {
    if (focused) focusedOnce = true;
    else if (focusedOnce && !dialogOpen) void close();
  });
  await win.show();
  await win.setFocus();
}

init().catch((err) => {
  console.error(err);
  void close();
});
