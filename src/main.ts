import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type DictationStatus = "idle" | "recording" | "transcribing" | "error";

interface StatusSnapshot {
  status: DictationStatus;
  model_path: string;
  model_loaded: boolean;
  last_transcript: string | null;
  last_error: string | null;
  log: string[];
}

const els = {
  badge: () => document.querySelector("#status-badge") as HTMLElement,
  modelLoaded: () => document.querySelector("#model-loaded") as HTMLElement,
  modelPath: () => document.querySelector("#model-path") as HTMLInputElement,
  transcript: () => document.querySelector("#transcript") as HTMLElement,
  log: () => document.querySelector("#log") as HTMLElement,
  error: () => document.querySelector("#error") as HTMLElement,
  btnToggle: () => document.querySelector("#btn-toggle") as HTMLButtonElement,
  btnCancel: () => document.querySelector("#btn-cancel") as HTMLButtonElement,
  btnSave: () => document.querySelector("#btn-save-path") as HTMLButtonElement,
  btnLoad: () => document.querySelector("#btn-load") as HTMLButtonElement,
};

function applyStatus(s: StatusSnapshot) {
  const badge = els.badge();
  badge.textContent = s.status;
  badge.className = `badge ${s.status}`;

  els.modelLoaded().textContent = s.model_loaded ? "loaded" : "not loaded";
  els.modelPath().value = s.model_path;
  els.transcript().textContent = s.last_transcript ?? "—";
  els.log().textContent = s.log.join("\n");

  const err = els.error();
  if (s.last_error) {
    err.hidden = false;
    err.textContent = s.last_error;
  } else {
    err.hidden = true;
    err.textContent = "";
  }

  const busy = s.status === "transcribing";
  els.btnToggle().disabled = busy;
  els.btnCancel().disabled = s.status !== "recording";
  els.btnToggle().textContent =
    s.status === "recording" ? "Stop & transcribe" : "Start / stop dictation";
}

async function refresh() {
  const s = await invoke<StatusSnapshot>("get_status");
  applyStatus(s);
}

window.addEventListener("DOMContentLoaded", async () => {
  els.btnToggle().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("toggle_dictation");
      applyStatus(s);
    } catch (e) {
      console.error(e);
      await refresh();
    }
  });

  els.btnCancel().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("cancel_dictation");
      applyStatus(s);
    } catch (e) {
      console.error(e);
      await refresh();
    }
  });

  els.btnSave().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_model_path", {
        path: els.modelPath().value,
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
    }
  });

  els.btnLoad().addEventListener("click", async () => {
    try {
      els.btnLoad().disabled = true;
      const s = await invoke<StatusSnapshot>("load_model");
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    } finally {
      els.btnLoad().disabled = false;
    }
  });

  await listen<StatusSnapshot>("dictation-status", (event) => {
    applyStatus(event.payload);
  });

  try {
    await refresh();
  } catch (e) {
    console.error("Failed to load status", e);
  }
});
