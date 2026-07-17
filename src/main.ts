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
  id: string;
  from: string;
  to: string;
  origin: "manual" | "tuning";
  edit_state: "unmodified" | "modified_after_verification";
  verified_fingerprints: {
    fingerprint: string;
    verified_at_ms: number;
  }[];
  version: number;
}

interface DictionaryMigrationConflict {
  id: string;
  canonical_from: string;
  choices: DictEntry[];
}

interface Snippet {
  cue: string;
  expansion: string;
}

interface HistoryEntry {
  id: string;
  at_ms: number;
  kind: string;
  text: string;
  raw?: string | null;
}

interface MicDeviceInfo {
  name: string;
  is_default: boolean;
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
  dictionary_revision: number;
  dictionary_conflicts: DictionaryMigrationConflict[];
  snippets_path: string;
  snippets: Snippet[];
  history_path: string;
  history_enabled: boolean;
  history_max: number;
  history: HistoryEntry[];
  clipboard_restore: boolean;
  /** Leading/trailing silence trim before Whisper. Default on. */
  silence_trim: boolean;
  /** macOS: hide Dock on next launch (Accessory). Default off. */
  menu_bar_only: boolean;
  /** True only on macOS builds — hide the Settings control elsewhere. */
  menu_bar_only_available: boolean;
  /** Preferred mic name; null = system default. */
  input_device_name: string | null;
  /** Open-time mic label from last recording start. */
  last_input_device_label: string | null;
  /** Backend-computed fallback notice (display only; do not re-parse labels). */
  last_mic_fallback_notice: string | null;
  last_transcript: string | null;
  last_raw_transcript: string | null;
  last_error: string | null;
  log: string[];
  session_kind: string;
  /** Compile-time STT backend: metal | cuda | vulkan | cpu. */
  stt_accel: string;
  /** Soft hint: Apple Silicon + CPU-only (rebuild with Metal). */
  show_metal_rebuild_hint: boolean;
  /** True only when both OS global shortcuts registered successfully. */
  global_hotkeys_ok: boolean;
  /** Dictation OS shortcut registered (independent of command). */
  dictation_hotkey_ok?: boolean;
  /** Command Mode OS shortcut registered (independent of dictation). */
  command_hotkey_ok?: boolean;
  /** Linux session probe: x11 | wayland | other | unknown. */
  linux_session: string;
  /** When true, first-run setup checklist should not auto-show. */
  onboarding_dismissed: boolean;
  /** Compile-time host: macos | linux | other. */
  host_os: string;
  /**
   * Failure-time permissions help code from last_error:
   * microphone | accessibility | model (independent of onboarding_dismissed).
   */
  permissions_help: string | null;
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
  hotkeysUnavailableChip: () =>
    document.querySelector("#hotkeys-unavailable-chip") as HTMLElement,
  hotkeysUnavailableBanner: () =>
    document.querySelector("#hotkeys-unavailable-banner") as HTMLElement,
  modelPath: () => document.querySelector("#model-path") as HTMLInputElement,
  sttAccelLabel: () =>
    document.querySelector("#stt-accel-label") as HTMLElement,
  sttAccelMetalHint: () =>
    document.querySelector("#stt-accel-metal-hint") as HTMLElement,
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
  historyList: () => document.querySelector("#history-list") as HTMLUListElement,
  historyPath: () => document.querySelector("#history-path") as HTMLElement,
  historyEnabled: () =>
    document.querySelector("#history-enabled") as HTMLInputElement,
  historyMaxLabel: () =>
    document.querySelector("#history-max-label") as HTMLElement,
  btnClearHistory: () =>
    document.querySelector("#btn-clear-history") as HTMLButtonElement,
  clipboardRestore: () =>
    document.querySelector("#clipboard-restore") as HTMLInputElement,
  silenceTrim: () =>
    document.querySelector("#silence-trim") as HTMLInputElement,
  menuBarOnlySection: () =>
    document.querySelector("#menu-bar-only-section") as HTMLElement,
  menuBarOnly: () =>
    document.querySelector("#menu-bar-only") as HTMLInputElement,
  micDevice: () => document.querySelector("#mic-device") as HTMLSelectElement,
  btnMicRefresh: () =>
    document.querySelector("#btn-mic-refresh") as HTMLButtonElement,
  btnMicSave: () => document.querySelector("#btn-mic-save") as HTMLButtonElement,
  micStatus: () => document.querySelector("#mic-status") as HTMLElement,
  setupChecklist: () =>
    document.querySelector("#setup-checklist") as HTMLElement,
  setupMacos: () => document.querySelector("#setup-macos") as HTMLElement,
  setupLinux: () => document.querySelector("#setup-linux") as HTMLElement,
  btnSetupDismiss: () =>
    document.querySelector("#btn-setup-dismiss") as HTMLButtonElement,
  btnShowSetup: () =>
    document.querySelector("#btn-show-setup") as HTMLButtonElement,
  btnOpenMicPrivacy: () =>
    document.querySelector("#btn-open-mic-privacy") as HTMLButtonElement,
  btnOpenAxPrivacy: () =>
    document.querySelector("#btn-open-ax-privacy") as HTMLButtonElement,
  btnSetupFocusModel: () =>
    document.querySelector("#btn-setup-focus-model") as HTMLButtonElement,
  btnSetupFocusModelLinux: () =>
    document.querySelector(
      "#btn-setup-focus-model-linux",
    ) as HTMLButtonElement,
  permissionsFailureHelp: () =>
    document.querySelector("#permissions-failure-help") as HTMLElement,
  pfhTitle: () => document.querySelector("#pfh-title") as HTMLElement,
  pfhBody: () => document.querySelector("#pfh-body") as HTMLElement,
  pfhPath: () => document.querySelector("#pfh-path") as HTMLElement,
  pfhClipboard: () => document.querySelector("#pfh-clipboard") as HTMLElement,
  btnPfhDismiss: () =>
    document.querySelector("#btn-pfh-dismiss") as HTMLButtonElement,
  btnPfhOpenMic: () =>
    document.querySelector("#btn-pfh-open-mic") as HTMLButtonElement,
  btnPfhOpenAx: () =>
    document.querySelector("#btn-pfh-open-ax") as HTMLButtonElement,
  btnPfhFocusModel: () =>
    document.querySelector("#btn-pfh-focus-model") as HTMLButtonElement,
  btnPfhShowChecklist: () =>
    document.querySelector("#btn-pfh-show-checklist") as HTMLButtonElement,
};

/**
 * When true, checklist was opened from Settings and stays visible until the
 * user dismisses — even if `onboarding_dismissed` is already true.
 */
let setupForcedOpen = false;
/** Last known dismiss flag from status (for auto-show decisions). */
let onboardingDismissed = false;
/** Host OS from status for macOS vs Linux checklist bodies. */
let hostOs: string = "other";
/**
 * When the user dismisses failure-time help, hide until permissions_help changes
 * (or clears). Does not depend on onboarding_dismissed.
 */
let pfhUserDismissedKind: string | null = null;
/** Last permissions_help code applied from status (for dismiss tracking). */
let lastPermissionsHelpKind: string | null = null;

/** Same checklist guidance, used for failure-time help (spec §3.3). */
type PermissionsHelpKind = "microphone" | "accessibility" | "model";

interface PermissionsHelpCopy {
  title: string;
  body: string;
  /** macOS manual path; null when not applicable. */
  pathMac: string | null;
  bodyLinux: string;
  showClipboard: boolean;
}

const PERMISSIONS_HELP_COPY: Record<PermissionsHelpKind, PermissionsHelpCopy> = {
  microphone: {
    title: "Microphone access",
    body: "So we can hear you. Grant mic access for EagleScribe.",
    pathMac:
      "System Settings → Privacy & Security → Microphone → enable EagleScribe",
    bodyLinux:
      "Capture needs mic access (PipeWire / PulseAudio on most desktops). Grant when the OS prompts, or check session audio settings.",
    showClipboard: false,
  },
  accessibility: {
    title: "Paste into other apps",
    body: "So we can paste into other apps (clipboard + simulated paste). Enable Accessibility for EagleScribe.",
    pathMac:
      "System Settings → Privacy & Security → Accessibility → enable EagleScribe",
    bodyLinux:
      "Paste reliability depends on session type (X11 vs Wayland) and packages. On failure, text stays on the clipboard for manual paste.",
    showClipboard: true,
  },
  model: {
    title: "Whisper model",
    body: "Local speech-to-text needs a ggml model file. Set path under Settings → Whisper model, then Load. Or run npm run model:download (see README).",
    pathMac: null,
    bodyLinux:
      "Local speech-to-text needs a ggml model file. Set path under Settings → Whisper model, then Load. Or run npm run model:download (see README).",
    showClipboard: false,
  },
};

function parsePermissionsHelpKind(
  raw: string | null | undefined,
): PermissionsHelpKind | null {
  switch ((raw ?? "").toLowerCase()) {
    case "microphone":
    case "mic":
      return "microphone";
    case "accessibility":
    case "ax":
    case "paste":
      return "accessibility";
    case "model":
    case "whisper":
      return "model";
    default:
      return null;
  }
}

/**
 * Show failure-time permissions help for the current last_error classification.
 * Independent of onboarding_dismissed / checklist visibility.
 */
function applyPermissionsFailureHelp(s: StatusSnapshot) {
  const panel = els.permissionsFailureHelp();
  if (!panel) return;

  const kind = parsePermissionsHelpKind(s.permissions_help);
  lastPermissionsHelpKind = kind;

  if (!kind) {
    panel.hidden = true;
    pfhUserDismissedKind = null;
    return;
  }

  // User dismissed this specific failure kind; re-show if kind changes.
  if (pfhUserDismissedKind === kind) {
    panel.hidden = true;
    return;
  }

  const isMac = hostOs === "macos" || (s.host_os ?? "").toLowerCase() === "macos";
  const copy = PERMISSIONS_HELP_COPY[kind];
  els.pfhTitle().textContent = copy.title;
  els.pfhBody().textContent = isMac ? copy.body : copy.bodyLinux;

  const pathEl = els.pfhPath();
  if (isMac && copy.pathMac) {
    pathEl.hidden = false;
    pathEl.textContent = copy.pathMac;
  } else {
    pathEl.hidden = true;
    pathEl.textContent = "";
  }

  els.pfhClipboard().hidden = !copy.showClipboard;

  const showMic = kind === "microphone" && isMac;
  const showAx = kind === "accessibility" && isMac;
  const showModel = kind === "model";
  els.btnPfhOpenMic().hidden = !showMic;
  els.btnPfhOpenAx().hidden = !showAx;
  els.btnPfhFocusModel().hidden = !showModel;

  panel.hidden = false;
}

function dismissPermissionsFailureHelp() {
  pfhUserDismissedKind = lastPermissionsHelpKind;
  const panel = els.permissionsFailureHelp();
  if (panel) panel.hidden = true;
}

/** Preferred mic currently reflected in the select (from status). */
let currentInputDeviceName: string | null = null;
/** Device list last loaded from the host (empty until first fetch). */
let micDevicesLoaded = false;
/** Names from last successful enumeration (for missing-preferred cue). */
let lastMicDeviceNames: string[] = [];
/** Last enumeration error text (if any); Refresh / load clears or updates this. */
let micListError: string | null = null;
/** Fallback notice from last recording (from backend snapshot; display only). */
let micFallbackNotice: string | null = null;

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

function renderDictionary(
  entries: DictEntry[],
  conflicts: DictionaryMigrationConflict[],
  path: string,
) {
  els.dictPath().textContent = path || "—";
  const list = els.dictList();
  list.innerHTML = "";
  if (!entries.length && !conflicts.length) {
    const li = document.createElement("li");
    li.className = "dict-empty";
    li.textContent = "No entries yet.";
    list.appendChild(li);
    return;
  }
  for (const conflict of conflicts) {
    const li = document.createElement("li");
    li.className = "dict-item";

    const text = document.createElement("span");
    text.className = "dict-text";
    text.textContent = `Choose the mapping for “${conflict.canonical_from}”: `;
    li.appendChild(text);

    for (const choice of conflict.choices) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "secondary";
      btn.textContent = `${choice.from} → ${choice.to}`;
      btn.addEventListener("click", async () => {
        try {
          const s = await invoke<StatusSnapshot>(
            "dictionary_resolve_migration_conflict",
            {
              conflictId: conflict.id,
              selectedEntryId: choice.id,
            },
          );
          applyStatus(s);
        } catch (e) {
          alert(String(e));
        }
      });
      li.appendChild(btn);
    }
    list.appendChild(li);
  }
  for (const entry of entries) {
    const li = document.createElement("li");
    li.className = "dict-item";

    const text = document.createElement("span");
    text.className = "dict-text";
    const lifecycle =
      entry.origin === "tuning"
        ? entry.edit_state === "modified_after_verification"
          ? "Tuning · explicitly edited"
          : entry.verified_fingerprints.length
            ? `Tuning · scoped to ${entry.verified_fingerprints.length} verified model${entry.verified_fingerprints.length === 1 ? "" : "s"}`
            : "Tuning · needs verification"
        : "Manual";
    text.innerHTML = `<code>${escapeHtml(entry.from)}</code> → <strong>${escapeHtml(entry.to)}</strong><span class="dict-meta">${escapeHtml(lifecycle)}</span>`;

    const edit = document.createElement("button");
    edit.type = "button";
    edit.className = "secondary";
    edit.textContent = "Edit";
    edit.addEventListener("click", async () => {
      const from = window.prompt("What EagleScribe should match", entry.from);
      if (from === null) return;
      const to = window.prompt("Preferred text", entry.to);
      if (to === null) return;
      try {
        const s = await invoke<StatusSnapshot>("dictionary_edit", {
          entryId: entry.id,
          expectedVersion: entry.version,
          from,
          to,
        });
        applyStatus(s);
      } catch (e) {
        alert(String(e));
      }
    });

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "secondary dict-remove";
    remove.textContent = "Remove";
    remove.addEventListener("click", async () => {
      try {
        const s = await invoke<StatusSnapshot>("dictionary_remove_entry", {
          entryId: entry.id,
          expectedVersion: entry.version,
        });
        applyStatus(s);
      } catch (e) {
        alert(String(e));
      }
    });

    li.appendChild(text);
    li.appendChild(edit);
    li.appendChild(remove);
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

function formatHistoryTime(atMs: number): string {
  if (!atMs) return "—";
  try {
    return new Date(atMs).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "—";
  }
}

function renderHistory(
  entries: HistoryEntry[],
  path: string,
  enabled: boolean,
  max: number,
) {
  els.historyPath().textContent = path || "—";
  els.historyMaxLabel().textContent = String(max || 50);
  const checkbox = els.historyEnabled();
  if (document.activeElement !== checkbox) {
    checkbox.checked = enabled;
  }
  els.btnClearHistory().disabled = !entries.length;

  const list = els.historyList();
  list.innerHTML = "";
  if (!enabled && !entries.length) {
    const li = document.createElement("li");
    li.className = "dict-empty";
    li.textContent = "History is off. Enable Save to retain transcripts.";
    list.appendChild(li);
    return;
  }
  if (!entries.length) {
    const li = document.createElement("li");
    li.className = "dict-empty";
    li.textContent = "No history yet. Dictate or run Command Mode.";
    list.appendChild(li);
    return;
  }
  for (const entry of entries) {
    const li = document.createElement("li");
    li.className = "history-item";

    const meta = document.createElement("div");
    meta.className = "history-meta";
    const kind = document.createElement("span");
    kind.className = `history-kind ${entry.kind === "command" ? "command" : ""}`;
    kind.textContent = entry.kind || "dictation";
    const when = document.createElement("span");
    when.textContent = formatHistoryTime(entry.at_ms);
    meta.appendChild(kind);
    meta.appendChild(when);

    const text = document.createElement("p");
    text.className = "history-text";
    text.textContent = entry.text;

    li.appendChild(meta);
    li.appendChild(text);

    if (entry.raw && entry.raw !== entry.text) {
      const raw = document.createElement("p");
      raw.className = "history-raw";
      raw.textContent = entry.raw;
      li.appendChild(raw);
    }

    list.appendChild(li);
  }
}

/**
 * Build mic select options: System default + host devices.
 * `selection` is the value to select after rebuild (may differ from saved preferred
 * when Refresh preserves an unsaved dirty choice).
 */
function populateMicSelect(
  devices: MicDeviceInfo[],
  preferred: string | null,
  selection: string,
) {
  const select = els.micDevice();
  const prevFocus = document.activeElement === select;
  const names = devices.map((d) => d.name);
  lastMicDeviceNames = names;

  select.innerHTML = "";
  const def = document.createElement("option");
  def.value = "";
  def.textContent = "System default";
  select.appendChild(def);

  for (const d of devices) {
    const opt = document.createElement("option");
    opt.value = d.name;
    opt.textContent = d.is_default ? `${d.name} (host default)` : d.name;
    select.appendChild(opt);
  }

  // Ensure the intended selection exists (unplugged preferred or dirty choice).
  const ensureOption = (value: string) => {
    if (!value || names.includes(value)) return;
    if (Array.from(select.options).some((o) => o.value === value)) return;
    const missing = document.createElement("option");
    missing.value = value;
    missing.textContent = `${value} (not found)`;
    select.appendChild(missing);
  };
  ensureOption(selection);
  // Keep saved preferred visible if different from selection and also missing.
  if (preferred && preferred !== selection) {
    ensureOption(preferred);
  }

  select.value = selection;
  // If value didn't stick (shouldn't happen), force system default.
  if (select.value !== selection) {
    select.value = "";
  }
  micDevicesLoaded = true;
  refreshMicStatusHint(preferred);
  if (prevFocus) select.focus();
}

/**
 * Re-enumerate host input devices (Settings Refresh / initial open).
 * Invokes `list_mic_devices` each time — no device cache on the backend.
 *
 * @param preferred saved preference (for missing-device cue)
 * @param selection value to keep selected after rebuild (defaults to preferred)
 */
async function loadMicDevices(
  preferred: string | null,
  selection?: string | null,
) {
  const statusEl = els.micStatus();
  const btn = els.btnMicRefresh();
  const prevDisabled = btn.disabled;
  const keep = selection !== undefined ? (selection ?? "") : (preferred ?? "");
  btn.disabled = true;
  try {
    // Fresh enumeration every call = refresh path (acceptance: new mics without restart).
    const devices = await invoke<MicDeviceInfo[]>("list_mic_devices");
    micListError = null;
    populateMicSelect(devices ?? [], preferred, keep);
  } catch (e) {
    // Keep System default option so Settings stays usable.
    micListError = `Could not list microphones: ${String(e)}`;
    populateMicSelect([], preferred, keep);
    statusEl.hidden = false;
    statusEl.textContent = micListError;
  } finally {
    btn.disabled = prevDisabled;
  }
}

/** Refresh #mic-status from list error, last-recording fallback, or missing preferred. */
function refreshMicStatusHint(preferred: string | null) {
  const statusEl = els.micStatus();
  if (micListError) {
    statusEl.hidden = false;
    statusEl.textContent = micListError;
    return;
  }
  if (micFallbackNotice) {
    statusEl.hidden = false;
    statusEl.textContent = micFallbackNotice;
    return;
  }
  const want = preferred ?? "";
  if (!want || !micDevicesLoaded) {
    statusEl.hidden = true;
    statusEl.textContent = "";
    return;
  }
  if (!lastMicDeviceNames.includes(want)) {
    statusEl.hidden = false;
    statusEl.textContent = `Preferred mic “${want}” not found — next recording uses system default.`;
    return;
  }
  statusEl.hidden = true;
  statusEl.textContent = "";
}

function applyMicPreference(preferred: string | null) {
  const select = els.micDevice();
  // Unsaved choice: select differs from last known saved preference.
  const dirty =
    micDevicesLoaded && select.value !== (currentInputDeviceName ?? "");
  currentInputDeviceName = preferred;

  if (!micDevicesLoaded) {
    // Options not loaded yet; loadMicDevices will set value.
    return;
  }
  // Don't clobber a focused or dirty select on status push (e.g. recording).
  if (document.activeElement === select || dirty) {
    refreshMicStatusHint(preferred);
    return;
  }

  const want = preferred ?? "";
  // Ensure preferred option exists (may have been unplugged).
  if (want && !Array.from(select.options).some((o) => o.value === want)) {
    const missing = document.createElement("option");
    missing.value = want;
    missing.textContent = `${want} (not found)`;
    select.appendChild(missing);
  }
  select.value = want;
  if (select.value !== want) select.value = "";
  refreshMicStatusHint(preferred);
}

/** Display label for compile-time STT acceleration (Settings read-only line). */
function formatSttAccel(raw: string | null | undefined): string {
  switch ((raw ?? "").toLowerCase()) {
    case "metal":
      return "Metal";
    case "cuda":
      return "CUDA";
    case "vulkan":
      return "Vulkan";
    case "cpu":
      return "CPU";
    case "":
      return "unknown";
    default:
      // Truthful fallback if backend sends an unexpected token.
      return raw as string;
  }
}

/** Surface backend-computed fallback notice in Settings mic-status (no label parsing). */
function applyMicFallbackFromStatus(s: StatusSnapshot) {
  const notice = s.last_mic_fallback_notice ?? null;
  micFallbackNotice = notice ? `Last recording: ${notice}` : null;
  refreshMicStatusHint(s.input_device_name ?? null);
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
  // Hotkey availability — prefer explicit flags, fall back to log lines, and
  // accept camelCase keys if the IPC layer ever renames fields.
  const rec = s as StatusSnapshot & Record<string, unknown>;
  const flag = (snake: string, camel: string): boolean | undefined => {
    const v = rec[snake] ?? rec[camel];
    if (v === true || v === false) return v;
    return undefined;
  };
  const bothFlag = flag("global_hotkeys_ok", "globalHotkeysOk");
  const dictFlag = flag("dictation_hotkey_ok", "dictationHotkeyOk");
  const cmdFlag = flag("command_hotkey_ok", "commandHotkeyOk");
  const logText = (s.log ?? []).join("\n");
  // Backend always logs this on successful grab — use as a hard source of truth
  // so a stale/missing flag cannot keep the scary banner up while Ctrl+Shift works.
  const dictFromLog =
    /Hotkey registration: dictation=ok/.test(logText) ||
    /Global hotkeys active:/.test(logText) ||
    /Global dictation hotkey observed live/.test(logText);
  const cmdFromLog =
    /Hotkey registration: command=ok/.test(logText) ||
    /Hotkey registration: dictation=ok.*command=ok/.test(logText) ||
    /Global hotkeys active:/.test(logText) ||
    /Global command hotkey observed live/.test(logText);
  const dictOk = dictFlag === true || bothFlag === true || dictFromLog;
  const cmdOk = cmdFlag === true || bothFlag === true || cmdFromLog;
  els.hotkeyHint().textContent = dictOk
    ? (HOTKEY_HINTS[hotkeyMode] ?? HOTKEY_HINTS.hold)
    : "ui only";
  updateHotkeyDisplays(
    s.dictation_hotkey ?? "Ctrl+Shift+Space",
    s.command_hotkey ?? "Ctrl+Shift+X",
  );
  const unavailChip = els.hotkeysUnavailableChip();
  const unavailBanner = els.hotkeysUnavailableBanner();
  // Top-bar chip: only when dictation itself is unavailable (primary path).
  // Note: `.chip { display: inline-flex }` would ignore the HTML `hidden`
  // attribute without the `[hidden] { display: none !important }` rule in CSS.
  if (unavailChip) {
    if (dictOk) {
      unavailChip.hidden = true;
    } else {
      unavailChip.hidden = false;
      unavailChip.textContent = "hotkeys unavailable — use window controls";
      unavailChip.title =
        "Dictation global hotkey could not be registered. Use Start/Stop in this window.";
    }
  }
  if (unavailBanner) {
    if (dictOk && cmdOk) {
      unavailBanner.hidden = true;
    } else if (dictOk && !cmdOk) {
      unavailBanner.hidden = false;
      unavailBanner.textContent =
        "Dictation hotkey is active. Command Mode global shortcut failed (maybe in use by another app) — use the Command Mode button in this window.";
    } else if (!dictOk && cmdOk) {
      unavailBanner.hidden = false;
      unavailBanner.textContent =
        "Command Mode hotkey is active, but the dictation global shortcut failed. Use Start dictation in this window.";
    } else {
      unavailBanner.hidden = false;
      unavailBanner.textContent =
        "Global hotkeys unavailable — use window controls. On Linux the current stack needs X11 for global shortcuts; pure Wayland often cannot register. In-window Start / Stop / Cancel still work.";
    }
  }
  els.modelPath().value = s.model_path;
  // Read-only compile-time STT acceleration (no runtime switch).
  els.sttAccelLabel().textContent = formatSttAccel(s.stt_accel);
  els.sttAccelMetalHint().hidden = s.show_metal_rebuild_hint !== true;
  if (document.activeElement !== els.llmUrl()) {
    els.llmUrl().value = s.llm_base_url ?? "";
  }
  if (document.activeElement !== els.llmModel()) {
    els.llmModel().value = s.llm_model ?? "";
  }
  els.transcript().textContent = s.last_transcript ?? "—";
  els.transcriptRaw().textContent = s.last_raw_transcript ?? "—";
  els.log().textContent = s.log.join("\n");
  renderDictionary(
    s.dictionary ?? [],
    s.dictionary_conflicts ?? [],
    s.dictionary_path ?? "",
  );
  renderSnippets(s.snippets ?? [], s.snippets_path ?? "");
  renderHistory(
    s.history ?? [],
    s.history_path ?? "",
    s.history_enabled ?? true,
    s.history_max ?? 50,
  );
  if (document.activeElement !== els.clipboardRestore()) {
    els.clipboardRestore().checked = s.clipboard_restore ?? true;
  }
  if (document.activeElement !== els.silenceTrim()) {
    els.silenceTrim().checked = s.silence_trim ?? true;
  }
  // macOS-only control; hidden on Linux/Windows (no dock-hide story).
  const mboAvailable = s.menu_bar_only_available === true;
  els.menuBarOnlySection().hidden = !mboAvailable;
  if (mboAvailable && document.activeElement !== els.menuBarOnly()) {
    els.menuBarOnly().checked = s.menu_bar_only ?? false;
  }
  applyMicPreference(s.input_device_name ?? null);
  applyMicFallbackFromStatus(s);
  applySetupChecklistFromStatus(s);

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
  // Contextual mic / Accessibility / model guidance (even if checklist dismissed).
  applyPermissionsFailureHelp(s);

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

type TabName = "settings" | "library" | "history" | "log";

function activateTab(name: TabName) {
  const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".tab"));
  const panels: Record<TabName, HTMLElement> = {
    settings: document.querySelector("#panel-settings") as HTMLElement,
    library: document.querySelector("#panel-library") as HTMLElement,
    history: document.querySelector("#panel-history") as HTMLElement,
    log: document.querySelector("#panel-log") as HTMLElement,
  };
  for (const tab of tabs) {
    const on = tab.dataset.tab === name;
    tab.classList.toggle("active", on);
    tab.setAttribute("aria-selected", on ? "true" : "false");
  }
  for (const [key, panel] of Object.entries(panels) as [TabName, HTMLElement][]) {
    const on = key === name;
    panel.classList.toggle("active", on);
    panel.hidden = !on;
  }
}

function setupTabs() {
  const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".tab"));
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      const name = tab.dataset.tab as TabName;
      if (name) activateTab(name);
    });
  }
}

/** Show/hide checklist body variants and panel based on dismiss + force flag. */
function syncSetupChecklistVisibility() {
  const panel = els.setupChecklist();
  const mac = els.setupMacos();
  const linux = els.setupLinux();
  if (!panel || !mac || !linux) return;

  const isMac = hostOs === "macos";
  mac.hidden = !isMac;
  // Linux notes for linux and other non-macOS (Windows out of scope; honest generic).
  linux.hidden = isMac;

  const shouldShow = setupForcedOpen || !onboardingDismissed;
  panel.hidden = !shouldShow;
}

function applySetupChecklistFromStatus(s: StatusSnapshot) {
  onboardingDismissed = s.onboarding_dismissed === true;
  hostOs = (s.host_os ?? "other").toLowerCase();
  // Auto-show when not dismissed; Settings force-open keeps it visible after dismiss.
  syncSetupChecklistVisibility();
}

function showSetupChecklist() {
  setupForcedOpen = true;
  syncSetupChecklistVisibility();
  // Scroll checklist into view without blocking the rest of the UI.
  const panel = els.setupChecklist();
  if (panel) {
    panel.hidden = false;
    panel.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
}

async function dismissSetupChecklist() {
  setupForcedOpen = false;
  try {
    const s = await invoke<StatusSnapshot>("set_onboarding_dismissed", {
      dismissed: true,
    });
    applyStatus(s);
  } catch (e) {
    // Still hide locally so the user is not stuck; next launch may re-show.
    onboardingDismissed = true;
    syncSetupChecklistVisibility();
    console.error(e);
  }
}

function focusModelSettings() {
  activateTab("settings");
  const input = els.modelPath();
  if (input) {
    input.focus();
    input.select();
    input.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
}

/**
 * Open a macOS privacy pane via Rust `open` (validated pane token).
 * Does not use the JS opener allowlist (which only covers mailto/tel/http(s) by default).
 * On failure, alert the always-visible manual path — never crash.
 */
async function openMacPrivacy(pane: "microphone" | "accessibility", label: string) {
  try {
    await invoke("open_macos_privacy_pane", { pane });
  } catch (e) {
    console.warn(`open ${label} settings failed`, e);
    alert(
      `Could not open System Settings automatically.\n\nUse the manual path under the ${label} row:\nSystem Settings → Privacy & Security → ${label}`,
    );
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  setupTabs();

  els.btnSetupDismiss().addEventListener("click", () => {
    void dismissSetupChecklist();
  });
  els.btnShowSetup().addEventListener("click", () => {
    showSetupChecklist();
  });
  els.btnOpenMicPrivacy()?.addEventListener("click", () => {
    void openMacPrivacy("microphone", "Microphone");
  });
  els.btnOpenAxPrivacy()?.addEventListener("click", () => {
    void openMacPrivacy("accessibility", "Accessibility");
  });
  els.btnSetupFocusModel()?.addEventListener("click", () => {
    focusModelSettings();
  });
  els.btnSetupFocusModelLinux()?.addEventListener("click", () => {
    focusModelSettings();
  });

  els.btnPfhDismiss()?.addEventListener("click", () => {
    dismissPermissionsFailureHelp();
  });
  els.btnPfhOpenMic()?.addEventListener("click", () => {
    void openMacPrivacy("microphone", "Microphone");
  });
  els.btnPfhOpenAx()?.addEventListener("click", () => {
    void openMacPrivacy("accessibility", "Accessibility");
  });
  els.btnPfhFocusModel()?.addEventListener("click", () => {
    focusModelSettings();
  });
  els.btnPfhShowChecklist()?.addEventListener("click", () => {
    showSetupChecklist();
  });

  els.historyEnabled().addEventListener("change", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_history_enabled", {
        enabled: els.historyEnabled().checked,
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  els.clipboardRestore().addEventListener("change", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_clipboard_restore", {
        enabled: els.clipboardRestore().checked,
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  els.silenceTrim().addEventListener("change", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_silence_trim", {
        enabled: els.silenceTrim().checked,
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  els.menuBarOnly().addEventListener("change", async () => {
    try {
      const s = await invoke<StatusSnapshot>("set_menu_bar_only", {
        enabled: els.menuBarOnly().checked,
      });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  els.btnClearHistory().addEventListener("click", async () => {
    if (!confirm("Clear all transcript history?")) return;
    try {
      const s = await invoke<StatusSnapshot>("clear_history");
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

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

  els.btnMicSave().addEventListener("click", async () => {
    const value = els.micDevice().value;
    const name = value.trim() === "" ? null : value;
    try {
      const s = await invoke<StatusSnapshot>("set_input_device", { name });
      applyStatus(s);
    } catch (e) {
      alert(String(e));
      await refresh();
    }
  });

  // Re-enumerate inputs so newly plugged mics appear without restarting.
  // Preserve an unsaved (dirty) dropdown choice across refresh.
  els.btnMicRefresh().addEventListener("click", async () => {
    try {
      const select = els.micDevice();
      const dirty =
        micDevicesLoaded && select.value !== (currentInputDeviceName ?? "");
      const selection = dirty ? select.value : (currentInputDeviceName ?? "");
      await loadMicDevices(currentInputDeviceName, selection);
    } catch (e) {
      console.error(e);
    }
  });

  await listen<StatusSnapshot>("dictation-status", (event) => {
    applyStatus(event.payload);
  });

  try {
    await refresh();
    await loadMicDevices(currentInputDeviceName);
  } catch (e) {
    console.error("Failed to load status", e);
  }
});
