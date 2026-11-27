// Clipboard history popup (like Windows' Win+V). Type to search; Enter copies the
// selected item and goes back to the app you were in, ready to paste.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Entry = {
  id: string;
  time: number;
  kind: "text" | "link" | "image" | "files";
  pinned: boolean;
  text?: string;
  long?: boolean;
  width?: number;
  height?: number;
  paths?: string[];
};
type Settings = {
  clip_limit: number;
  platform: string;
};

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const list = $("list");
const empty = $("empty");
const search = $<HTMLInputElement>("search");
const status = $("status");
const clearBtn = $("clear");
const tip = $("tip");
const template = $<HTMLTemplateElement>("row");

let isMac = navigator.userAgent.includes("Mac");
let entries: Entry[] = [];
let shown: Entry[] = [];
let selected = 0;
let defaultStatus = "";
let statusTimer = 0;
let emptyTitle = "Nada copiado ainda.";
let emptyText = "Copie um texto, link ou imagem e ele aparece aqui.";

const mod = () => (isMac ? "⌘" : "Ctrl+");
const KIND_LABEL: Record<Entry["kind"], string> = {
  text: "Texto",
  link: "Link",
  image: "Imagem",
  files: "Arquivos",
};

function showStatus(html: string, kind: "ok" | "error" | "" = "") {
  status.innerHTML = html;
  status.className = kind;
  clearTimeout(statusTimer);
  if (kind) statusTimer = window.setTimeout(() => showStatus(defaultStatus), 4000);
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

const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** What the search looks at. */
function haystack(e: Entry): string {
  if (e.kind === "files") return (e.paths ?? []).map(baseName).join("\n");
  if (e.kind === "image") return "imagem";
  return e.text ?? "";
}

/** Text with the matches of `query` highlighted, without trusting the text as HTML. */
function highlighted(text: string, query: string): Node[] {
  if (!query) return [document.createTextNode(text)];
  const nodes: Node[] = [];
  const lower = text.toLowerCase();
  let at = 0;
  for (let i = lower.indexOf(query); i >= 0 && nodes.length < 40; i = lower.indexOf(query, at)) {
    if (i > at) nodes.push(document.createTextNode(text.slice(at, i)));
    const mark = document.createElement("mark");
    mark.textContent = text.slice(i, i + query.length);
    nodes.push(mark);
    at = i + query.length;
  }
  nodes.push(document.createTextNode(text.slice(at)));
  return nodes;
}

/** For a long text with a match far down, start the preview near the match. */
function previewText(text: string, query: string): string {
  const compact = text.replace(/^\s+/, "");
  if (!query) return compact;
  const i = compact.toLowerCase().indexOf(query);
  if (i < 120) return compact;
  const start = compact.lastIndexOf("\n", i) + 1;
  return "…" + compact.slice(Math.max(start, i - 60));
}

function rows(): HTMLElement[] {
  return [...list.children] as HTMLElement[];
}

function render() {
  const query = search.value.trim().toLowerCase();
  // Pinned items first, then the newest.
  const ordered = [...entries.filter((e) => e.pinned), ...entries.filter((e) => !e.pinned)];
  shown = query ? ordered.filter((e) => haystack(e).toLowerCase().includes(query)) : ordered;

  list.replaceChildren(
    ...shown.map((e, index) => {
      const row = template.content.firstElementChild!.cloneNode(true) as HTMLElement;
      row.dataset.id = e.id;
      row.classList.add(e.kind);
      row.classList.toggle("pinned", e.pinned);
      const body = row.querySelector<HTMLElement>(".body")!;
      let extra = "";
      if (e.kind === "image") {
        const img = document.createElement("img");
        img.src = convertFileSrc(`c/${e.id}.jpg`, "kadro");
        img.alt = "";
        img.draggable = false;
        img.decoding = "async";
        body.append(img);
        extra = ` · ${e.width} × ${e.height}`;
      } else if (e.kind === "files") {
        const paths = e.paths ?? [];
        body.append(...highlighted(paths.map(baseName).join("\n"), query));
        body.title = paths.join("\n");
        extra = paths.length > 1 ? ` · ${paths.length} arquivos` : "";
      } else {
        body.append(...highlighted(previewText(e.text ?? "", query), query));
        if (e.long) extra = " · texto longo";
      }
      row.querySelector(".kind")!.textContent = KIND_LABEL[e.kind] + extra;
      const time = row.querySelector<HTMLElement>(".time")!;
      time.textContent = when(e.time);
      time.title = new Date(e.time).toLocaleString("pt-BR");
      row.querySelector(".num")!.textContent = index < 9 ? `${mod()}${index + 1}` : "";
      const pin = row.querySelector<HTMLElement>('[data-op="pin"]')!;
      pin.dataset.tip = `${e.pinned ? "Desafixar" : "Fixar no topo"} (${mod()}P)`;
      row.querySelector<HTMLElement>('[data-op="delete"]')!.dataset.tip = `Remover (${mod()}⌫)`.replace("Ctrl+⌫", "Ctrl+Del");
      return row;
    }),
  );

  const hasAny = entries.length > 0;
  list.hidden = shown.length === 0;
  empty.hidden = shown.length > 0;
  clearBtn.hidden = !entries.some((e) => !e.pinned);
  $("empty-title").textContent = hasAny ? "Nada encontrado." : emptyTitle;
  $("empty-text").textContent = hasAny ? "Tente outra palavra." : emptyText;
  select(Math.min(Math.max(selected, 0), shown.length - 1), false);
}

function select(index: number, scroll = true) {
  selected = index;
  rows().forEach((row, i) => row.classList.toggle("selected", i === index));
  if (scroll && index >= 0) rows()[index]?.scrollIntoView({ block: "nearest" });
}

async function load() {
  entries = await invoke<Entry[]>("clip_list");
  render();
}

async function use(index: number) {
  const entry = shown[index];
  if (!entry) return;
  try {
    // The window closes itself; nothing to do afterwards.
    await invoke("clip_use", { id: entry.id });
  } catch (err) {
    showStatus(String(err), "error");
    void load();
  }
}

async function togglePin(index: number) {
  const entry = shown[index];
  if (!entry) return;
  try {
    await invoke("clip_pin", { id: entry.id, pinned: !entry.pinned });
    entry.pinned = !entry.pinned;
    render();
    // Keep the same item selected after it moves.
    select(shown.indexOf(entry));
    showStatus(entry.pinned ? "Fixado no topo: fica até você remover" : "Desafixado", "ok");
  } catch (err) {
    showStatus(String(err), "error");
  }
}

async function remove(index: number) {
  const entry = shown[index];
  if (!entry) return;
  try {
    await invoke("clip_delete", { id: entry.id });
    entries = entries.filter((e) => e !== entry);
    render();
  } catch (err) {
    showStatus(String(err), "error");
  }
}

let clearTimer = 0;
async function clearAll() {
  // Two clicks: the first one only arms the button.
  if (!clearBtn.classList.contains("confirm")) {
    clearBtn.classList.add("confirm");
    clearBtn.dataset.tip = "Clique de novo para apagar";
    showTip(clearBtn);
    clearTimer = window.setTimeout(disarmClear, 3000);
    return;
  }
  disarmClear();
  hideTip();
  try {
    await invoke("clip_clear", { pinned: false });
    await load();
    showStatus("Histórico apagado (os fixados ficaram)", "ok");
  } catch (err) {
    showStatus(String(err), "error");
  }
}

function disarmClear() {
  clearTimeout(clearTimer);
  clearBtn.classList.remove("confirm");
  clearBtn.dataset.tip = "Apagar o histórico (os fixados ficam)";
}

async function close(restore: boolean) {
  await invoke("close_clipboard", { restore });
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
  const cmd = isMac ? e.metaKey : e.ctrlKey;
  const key = e.key.toLowerCase();
  if (e.key === "Escape") {
    e.preventDefault();
    if (search.value) {
      search.value = "";
      selected = 0;
      render();
    } else {
      void close(true);
    }
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!shown.length) return;
    const step = e.key === "ArrowDown" ? 1 : -1;
    select(Math.min(Math.max(selected + step, 0), shown.length - 1));
  } else if (e.key === "Enter") {
    e.preventDefault();
    void use(selected);
  } else if (cmd && /^[1-9]$/.test(e.key)) {
    e.preventDefault();
    void use(Number(e.key) - 1);
  } else if (cmd && key === "p") {
    e.preventDefault();
    void togglePin(selected);
  } else if (cmd && (e.key === "Backspace" || e.key === "Delete")) {
    e.preventDefault();
    void remove(selected);
  }
}

async function init() {
  const settings = await invoke<Settings>("get_settings");
  isMac = settings.platform === "macos";
  const k = (s: string) => `<kbd>${s}</kbd>`;
  const paste = isMac ? "⌘V" : "Ctrl+V";
  defaultStatus = `${k("Enter")} copia, aí é só ${k(paste)} · ${k(mod() + "P")} fixa`;
  if (settings.clip_limit === 0) {
    emptyTitle = "O histórico está desativado.";
    emptyText = "Ative em Preferências → Área de transferência.";
  }
  showStatus(defaultStatus);

  list.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const row = target.closest<HTMLElement>(".item");
    if (!row) return;
    const index = rows().indexOf(row);
    const op = target.closest<HTMLElement>("[data-op]")?.dataset.op;
    if (op === "pin") void togglePin(index);
    else if (op === "delete") void remove(index);
    else void use(index);
  });
  list.addEventListener("mousemove", (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>(".item");
    const index = row ? rows().indexOf(row) : -1;
    if (index >= 0 && index !== selected) select(index, false);
  });
  // Buttons must not take the focus from the search box.
  document.addEventListener("mousedown", (e) => {
    if ((e.target as HTMLElement).closest("button, .item")) e.preventDefault();
  });
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("mouseover", (e) => {
    const target = (e.target as HTMLElement).closest<HTMLElement>("[data-tip]");
    hideTip();
    if (target) tipTimer = window.setTimeout(() => showTip(target), 350);
  });
  list.addEventListener("scroll", hideTip, { passive: true });
  search.addEventListener("input", () => {
    selected = 0;
    render();
  });
  clearBtn.addEventListener("click", () => void clearAll());

  await listen("clipboard-changed", () => void load());
  await load();

  // Like a menu: the popup closes when you click anywhere else.
  const win = getCurrentWindow();
  let focusedOnce = false;
  await win.onFocusChanged(({ payload: focused }) => {
    if (focused) focusedOnce = true;
    else if (focusedOnce) void close(false);
  });
  await win.show();
  await win.setFocus();
  search.focus();
}

init().catch((err) => {
  console.error(err);
  void close(true);
});
