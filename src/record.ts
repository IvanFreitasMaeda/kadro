// Recording control bar: elapsed time, stop and discard. Kadro keeps it out of the
// recording itself.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { pretty } from "./keys";

declare global {
  interface Window {
    __KADRO_REC__?: { format: "mp4" | "gif" };
  }
}

const $ = (id: string) => document.getElementById(id)!;
const time = $("time");
const stopBtn = $("stop") as HTMLButtonElement;
const discardBtn = $("discard") as HTMLButtonElement;
const isMac = navigator.userAgent.includes("Mac");

const format = window.__KADRO_REC__?.format ?? "mp4";
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
}

function begin(elapsed: number) {
  if (done || started) return;
  started = Date.now() - elapsed;
  document.body.classList.add("recording");
  stopBtn.disabled = false;
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
// Buttons must not keep the focus: this bar shouldn't steal keys from other apps.
document.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button")) e.preventDefault();
});

async function init() {
  discardBtn.title = "Descartar gravação";
  await listen("recording-started", () => begin(0));
  await listen("recording-stopping", () => finishing("Salvando…"));
  // The recording may have started before this page finished loading.
  const elapsed = await invoke<number | null>("recording_elapsed");
  if (elapsed !== null) begin(elapsed);
  try {
    const s = await invoke<{ shortcut_record: string }>("get_settings");
    stopBtn.title = `Parar e salvar (${pretty(s.shortcut_record, isMac)})`;
  } catch {
    stopBtn.title = "Parar e salvar";
  }
}

void init();
