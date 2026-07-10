import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type DictationStatus = "idle" | "recording" | "transcribing" | "error";
type PolishMode = "smart" | "verbatim";

interface DictEntry {
  from: string;
  to: string;
}

interface Snippet {
  cue: string;
  expansion: string;
}

interface StatusSnapshot {
  status: DictationStatus;
  model_path: string;
  model_loaded: boolean;
  polish_mode: PolishMode;
  dictionary_path: string;
  dictionary: DictEntry[];
  snippets_path: string;
  snippets: Snippet[];
  last_transcript: string | null;
  last_raw_transcript: string | null;
  last_error: string | null;
  log: string[];
}

const els = {
  badge: () => document.querySelector("#status-badge") as HTMLElement,
  modelLoaded: () => document.querySelector("#model-loaded") as HTMLElement,
  polishMode: () => document.querySelector("#polish-mode") as HTMLElement,
  modelPath: () => document.querySelector("#model-path") as HTMLInputElement,
  transcript: () => document.querySelector("#transcript") as HTMLElement,
  transcriptRaw: () => document.querySelector("#transcript-raw") as HTMLElement,
  log: () => document.querySelector("#log") as HTMLElement,
  error: () => document.querySelector("#error") as HTMLElement,
  btnToggle: () => document.querySelector("#btn-toggle") as HTMLButtonElement,
  btnCancel: () => document.querySelector("#btn-cancel") as HTMLButtonElement,
  btnSave: () => document.querySelector("#btn-save-path") as HTMLButtonElement,
  btnLoad: () => document.querySelector("#btn-load") as HTMLButtonElement,
  polishSmart: () => document.querySelector("#polish-smart") as HTMLInputElement,
  polishVerbatim: () =>
    document.querySelector("#polish-verbatim") as HTMLInputElement,
  dictFrom: () => document.querySelector("#dict-from") as HTMLInputElement,
  dictTo: () => document.querySelector("#dict-to") as HTMLInputElement,
  dictList: () => document.querySelector("#dict-list") as HTMLUListElement,
  dictPath: () => document.querySelector("#dict-path") as HTMLElement,
  btnDictAdd: () => document.querySelector("#btn-dict-add") as HTMLButtonElement,
  snipCue: () => document.querySelector("#snip-cue") as HTMLInputElement,
  snipExpansion: () =>
    document.querySelector("#snip-expansion") as HTMLTextAreaElement,
  snipList: () => document.querySelector("#snip-list") as HTMLUListElement,
  snipPath: () => document.querySelector("#snip-path") as HTMLElement,
  btnSnipAdd: () => document.querySelector("#btn-snip-add") as HTMLButtonElement,
};

function renderDictionary(entries: DictEntry[], path: string) {
  els.dictPath().textContent = path || "—";
  const list = els.dictList();
  list.innerHTML = "";
  if (!entries.length) {
    const li = document.createElement("li");
    li.className = "dict-empty";
    li.textContent = "No entries yet.";
    list.appendChild(li);
    return;
  }
  for (const entry of entries) {
    const li = document.createElement("li");
    li.className = "dict-item";

    const text = document.createElement("span");
    text.className = "dict-text";
    text.innerHTML = `<code>${escapeHtml(entry.from)}</code> → <strong>${escapeHtml(entry.to)}</strong>`;

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "secondary dict-remove";
    btn.textContent = "Remove";
    btn.addEventListener("click", async () => {
      try {
        const s = await invoke<StatusSnapshot>("dictionary_remove", {
          from: entry.from,
        });
        applyStatus(s);
      } catch (e) {
        alert(String(e));
      }
    });

    li.appendChild(text);
    li.appendChild(btn);
    list.appendChild(li);
  }
}

function previewExpansion(s: string, max = 60): string {
  const oneLine = s.replace(/\s+/g, " ").trim();
  if (oneLine.length <= max) return oneLine;
  return oneLine.slice(0, max) + "…";
}

function renderSnippets(snippets: Snippet[], path: string) {
  els.snipPath().textContent = path || "—";
  const list = els.snipList();
  list.innerHTML = "";
  if (!snippets.length) {
    const li = document.createElement("li");
    li.className = "dict-empty";
    li.textContent = "No snippets yet.";
    list.appendChild(li);
    return;
  }
  for (const snip of snippets) {
    const li = document.createElement("li");
    li.className = "dict-item";

    const text = document.createElement("span");
    text.className = "dict-text";
    text.innerHTML = `<code>${escapeHtml(snip.cue)}</code><span class="snip-preview">${escapeHtml(previewExpansion(snip.expansion))}</span>`;

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "secondary dict-remove";
    btn.textContent = "Remove";
    btn.addEventListener("click", async () => {
      try {
        const s = await invoke<StatusSnapshot>("snippet_remove", {
          cue: snip.cue,
        });
        applyStatus(s);
      } catch (e) {
        alert(String(e));
      }
    });

    li.appendChild(text);
    li.appendChild(btn);
    list.appendChild(li);
  }
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function applyStatus(s: StatusSnapshot) {
  const badge = els.badge();
  badge.textContent = s.status;
  badge.className = `badge ${s.status}`;

  els.modelLoaded().textContent = s.model_loaded ? "loaded" : "not loaded";
  els.polishMode().textContent = s.polish_mode;
  els.modelPath().value = s.model_path;
  els.transcript().textContent = s.last_transcript ?? "—";
  els.transcriptRaw().textContent = s.last_raw_transcript ?? "—";
  els.log().textContent = s.log.join("\n");
  renderDictionary(s.dictionary ?? [], s.dictionary_path ?? "");
  renderSnippets(s.snippets ?? [], s.snippets_path ?? "");

  if (s.polish_mode === "verbatim") {
    els.polishVerbatim().checked = true;
  } else {
    els.polishSmart().checked = true;
  }

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

  const onPolishChange = async (mode: PolishMode) => {
    try {
      const s = await invoke<StatusSnapshot>("set_polish_mode", { mode });
      applyStatus(s);
    } catch (e) {
      console.error(e);
      await refresh();
    }
  };

  els.polishSmart().addEventListener("change", () => {
    if (els.polishSmart().checked) void onPolishChange("smart");
  });
  els.polishVerbatim().addEventListener("change", () => {
    if (els.polishVerbatim().checked) void onPolishChange("verbatim");
  });

  const addDict = async () => {
    const from = els.dictFrom().value.trim();
    const to = els.dictTo().value.trim();
    if (!from || !to) {
      alert("Enter both “what you say” and “write as”.");
      return;
    }
    try {
      const s = await invoke<StatusSnapshot>("dictionary_add", { from, to });
      applyStatus(s);
      els.dictFrom().value = "";
      els.dictTo().value = "";
      els.dictFrom().focus();
    } catch (e) {
      alert(String(e));
    }
  };

  els.btnDictAdd().addEventListener("click", () => void addDict());
  els.dictTo().addEventListener("keydown", (e) => {
    if (e.key === "Enter") void addDict();
  });
  els.dictFrom().addEventListener("keydown", (e) => {
    if (e.key === "Enter") els.dictTo().focus();
  });

  const addSnip = async () => {
    const cue = els.snipCue().value.trim();
    const expansion = els.snipExpansion().value;
    if (!cue || !expansion.trim()) {
      alert("Enter both a cue and expansion text.");
      return;
    }
    try {
      const s = await invoke<StatusSnapshot>("snippet_add", { cue, expansion });
      applyStatus(s);
      els.snipCue().value = "";
      els.snipExpansion().value = "";
      els.snipCue().focus();
    } catch (e) {
      alert(String(e));
    }
  };

  els.btnSnipAdd().addEventListener("click", () => void addSnip());

  await listen<StatusSnapshot>("dictation-status", (event) => {
    applyStatus(event.payload);
  });

  try {
    await refresh();
  } catch (e) {
    console.error("Failed to load status", e);
  }
});
