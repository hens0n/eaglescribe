import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type DictationStatus =
  | "idle"
  | "recording"
  | "transcribing"
  | "waiting_llm"
  | "error";

const STATUS_LABELS: Record<DictationStatus, string> = {
  idle: "idle",
  recording: "recording",
  transcribing: "transcribing",
  waiting_llm: "waiting llm",
  error: "error",
};
type PolishMode = "smart" | "verbatim";
type HotkeyMode = "hold" | "toggle";

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
  hotkey_mode: HotkeyMode;
  dictation_hotkey: string;
  command_hotkey: string;
  llm_base_url: string;
  llm_model: string;
  dictionary_path: string;
  dictionary: DictEntry[];
  snippets_path: string;
  snippets: Snippet[];
  last_transcript: string | null;
  last_raw_transcript: string | null;
  last_error: string | null;
  log: string[];
  session_kind: string;
}

const HOTKEY_HINTS: Record<HotkeyMode, string> = {
  hold: "hold",
  toggle: "toggle",
};

type CaptureTarget = "dictation" | "command" | null;

let captureTarget: CaptureTarget = null;
/** Latest known bindings (for partial updates while capturing). */
let currentDictationHotkey = "Ctrl+Shift+Space";
let currentCommandHotkey = "Ctrl+Shift+X";

const els = {
  badge: () => document.querySelector("#status-badge") as HTMLElement,
  modelLoaded: () => document.querySelector("#model-loaded") as HTMLElement,
  polishMode: () => document.querySelector("#polish-mode") as HTMLElement,
  hotkeyMode: () => document.querySelector("#hotkey-mode") as HTMLElement,
  hotkeyHint: () => document.querySelector("#hotkey-hint") as HTMLElement,
  hotkeyHold: () => document.querySelector("#hotkey-hold") as HTMLInputElement,
  hotkeyToggle: () => document.querySelector("#hotkey-toggle") as HTMLInputElement,
  statusDictationHotkey: () =>
    document.querySelector("#status-dictation-hotkey") as HTMLElement,
  statusCommandHotkey: () =>
    document.querySelector("#status-command-hotkey") as HTMLElement,
  dictationHotkeyDisplay: () =>
    document.querySelector("#dictation-hotkey-display") as HTMLElement,
  commandHotkeyDisplay: () =>
    document.querySelector("#command-hotkey-display") as HTMLElement,
  btnChangeDictation: () =>
    document.querySelector("#btn-change-dictation") as HTMLButtonElement,
  btnChangeCommand: () =>
    document.querySelector("#btn-change-command") as HTMLButtonElement,
  btnResetHotkeys: () =>
    document.querySelector("#btn-reset-hotkeys") as HTMLButtonElement,
  hotkeyCaptureStatus: () =>
    document.querySelector("#hotkey-capture-status") as HTMLElement,
  modelPath: () => document.querySelector("#model-path") as HTMLInputElement,
  transcript: () => document.querySelector("#transcript") as HTMLElement,
  transcriptRaw: () => document.querySelector("#transcript-raw") as HTMLElement,
  log: () => document.querySelector("#log") as HTMLElement,
  error: () => document.querySelector("#error") as HTMLElement,
  btnToggle: () => document.querySelector("#btn-toggle") as HTMLButtonElement,
  btnCommand: () => document.querySelector("#btn-command") as HTMLButtonElement,
  btnCancel: () => document.querySelector("#btn-cancel") as HTMLButtonElement,
  btnSave: () => document.querySelector("#btn-save-path") as HTMLButtonElement,
  btnLoad: () => document.querySelector("#btn-load") as HTMLButtonElement,
  llmUrl: () => document.querySelector("#llm-url") as HTMLInputElement,
  llmModel: () => document.querySelector("#llm-model") as HTMLInputElement,
  btnLlmSave: () => document.querySelector("#btn-llm-save") as HTMLButtonElement,
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

/** Map a KeyboardEvent to a global-hotkey string (modifiers + e.code). */
function eventToHotkeyString(e: KeyboardEvent): string | null {
  const code = e.code;
  if (
    code === "ControlLeft" ||
    code === "ControlRight" ||
    code === "ShiftLeft" ||
    code === "ShiftRight" ||
    code === "AltLeft" ||
    code === "AltRight" ||
    code === "MetaLeft" ||
    code === "MetaRight" ||
    code === "OSLeft" ||
    code === "OSRight"
  ) {
    return null;
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.metaKey) parts.push("Cmd");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (parts.length === 0) return null;

  // e.code is already "KeyX", "Space", "Digit1", "F5", … — accepted by the parser.
  parts.push(code);
  return parts.join("+");
}

function setCaptureUi(target: CaptureTarget) {
  captureTarget = target;
  const status = els.hotkeyCaptureStatus();
  const dDisp = els.dictationHotkeyDisplay();
  const cDisp = els.commandHotkeyDisplay();
  dDisp.classList.toggle("listening", target === "dictation");
  cDisp.classList.toggle("listening", target === "command");
  els.btnChangeDictation().disabled = target !== null;
  els.btnChangeCommand().disabled = target !== null;
  els.btnResetHotkeys().disabled = target !== null;

  if (!target) {
    status.hidden = true;
    status.textContent = "";
    return;
  }
  status.hidden = false;
  status.textContent =
    target === "dictation"
      ? "Listening for dictation hotkey… (Esc to cancel)"
      : "Listening for Command Mode hotkey… (Esc to cancel)";
}

function stopCapture() {
  setCaptureUi(null);
}

async function applyHotkeys(dictation: string, command: string) {
  const s = await invoke<StatusSnapshot>("set_hotkeys", { dictation, command });
  applyStatus(s);
}

function updateHotkeyDisplays(dictation: string, command: string) {
  currentDictationHotkey = dictation;
  currentCommandHotkey = command;
  els.statusDictationHotkey().textContent = dictation;
  els.statusCommandHotkey().textContent = command;
  els.dictationHotkeyDisplay().textContent = dictation;
  els.commandHotkeyDisplay().textContent = command;
}

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
  const status = (s.status ?? "idle") as DictationStatus;
  badge.textContent = STATUS_LABELS[status] ?? status;
  badge.className = `badge ${status}`;

  els.modelLoaded().textContent = s.model_loaded ? "loaded" : "not loaded";
  els.polishMode().textContent = s.polish_mode;
  const hotkeyMode = s.hotkey_mode ?? "hold";
  els.hotkeyMode().textContent = hotkeyMode;
  els.hotkeyHint().textContent = HOTKEY_HINTS[hotkeyMode] ?? HOTKEY_HINTS.hold;
  updateHotkeyDisplays(
    s.dictation_hotkey ?? "Ctrl+Shift+Space",
    s.command_hotkey ?? "Ctrl+Shift+X",
  );
  els.modelPath().value = s.model_path;
  if (document.activeElement !== els.llmUrl()) {
    els.llmUrl().value = s.llm_base_url ?? "";
  }
  if (document.activeElement !== els.llmModel()) {
    els.llmModel().value = s.llm_model ?? "";
  }
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

  if (hotkeyMode === "toggle") {
    els.hotkeyToggle().checked = true;
  } else {
    els.hotkeyHold().checked = true;
  }

  const err = els.error();
  if (s.last_error) {
    err.hidden = false;
    err.textContent = s.last_error;
  } else {
    err.hidden = true;
    err.textContent = "";
  }

  const busy = status === "transcribing" || status === "waiting_llm";
  const recording = status === "recording";
  const isCommand = s.session_kind === "command";
  els.btnToggle().disabled = busy || (recording && isCommand);
  els.btnCommand().disabled = busy || (recording && !isCommand);
  els.btnCancel().disabled = !recording;
  els.btnToggle().textContent =
    recording && !isCommand
      ? "Stop & transcribe (toggle)"
      : "Start dictation (toggle)";
  els.btnCommand().textContent =
    recording && isCommand
      ? "Stop command (run LLM)"
      : status === "waiting_llm"
        ? "Waiting on LLM…"
        : "Command Mode";
}

async function refresh() {
  const s = await invoke<StatusSnapshot>("get_status");
  applyStatus(s);
}

function setupTabs() {
  const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".tab"));
  const panels = {
    settings: document.querySelector("#panel-settings") as HTMLElement,
    library: document.querySelector("#panel-library") as HTMLElement,
    log: document.querySelector("#panel-log") as HTMLElement,
  };

  const activate = (name: keyof typeof panels) => {
    for (const tab of tabs) {
      const on = tab.dataset.tab === name;
      tab.classList.toggle("active", on);
      tab.setAttribute("aria-selected", on ? "true" : "false");
    }
    for (const [key, panel] of Object.entries(panels)) {
      const on = key === name;
      panel.classList.toggle("active", on);
      panel.hidden = !on;
    }
  };

  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      const name = tab.dataset.tab as keyof typeof panels;
      if (name && panels[name]) activate(name);
    });
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  setupTabs();

  els.btnToggle().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("toggle_dictation");
      applyStatus(s);
    } catch (e) {
      console.error(e);
      await refresh();
    }
  });

  els.btnCommand().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("toggle_command_mode");
      applyStatus(s);
    } catch (e) {
      console.error(e);
      alert(String(e));
      await refresh();
    }
  });

  els.btnLlmSave().addEventListener("click", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_llm_settings", {
        baseUrl: els.llmUrl().value,
        model: els.llmModel().value,
        apiKey: "",
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
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

  const onHotkeyModeChange = async (mode: HotkeyMode) => {
    try {
      const s = await invoke<StatusSnapshot>("set_hotkey_mode", { mode });
      applyStatus(s);
    } catch (e) {
      console.error(e);
      await refresh();
    }
  };

  els.hotkeyHold().addEventListener("change", () => {
    if (els.hotkeyHold().checked) void onHotkeyModeChange("hold");
  });
  els.hotkeyToggle().addEventListener("change", () => {
    if (els.hotkeyToggle().checked) void onHotkeyModeChange("toggle");
  });

  els.btnChangeDictation().addEventListener("click", () => {
    setCaptureUi("dictation");
  });
  els.btnChangeCommand().addEventListener("click", () => {
    setCaptureUi("command");
  });
  els.btnResetHotkeys().addEventListener("click", async () => {
    stopCapture();
    try {
      const s = await invoke<StatusSnapshot>("reset_hotkeys");
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  window.addEventListener(
    "keydown",
    (e) => {
      if (!captureTarget) return;
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        stopCapture();
        return;
      }

      const combo = eventToHotkeyString(e);
      if (!combo) return;

      const target = captureTarget;
      stopCapture();

      void (async () => {
        try {
          if (target === "dictation") {
            await applyHotkeys(combo, currentCommandHotkey);
          } else if (target === "command") {
            await applyHotkeys(currentDictationHotkey, combo);
          }
        } catch (err) {
          alert(String(err));
          await refresh();
        }
      })();
    },
    true,
  );

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
