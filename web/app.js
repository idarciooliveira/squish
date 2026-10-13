// squish web UI: a client-side queue in front of POST /api/compress (ADR-0007, ADR-0018).

import { downloadZip } from "./vendor/client-zip.js";

const IN_FLIGHT = 4; // uploads at once (PRD FR-19)
const MAX_TRIES = 8; // per file, counting 429 retries
// Extensions a folder walk picks up, as in the CLI (ADR-0019). Files picked one by one are always sent.
const IMAGE_EXT = /\.(png|jpe?g|webp|avif|gif|apng)$/i;

const $ = (id) => document.getElementById(id);
const items = [];
const queue = [];
let running = 0;
let limits = { max_body_bytes: Infinity };

// --- server info -------------------------------------------------------------

fetch("api/info")
  .then((r) => r.json())
  .then((info) => {
    limits = info.limits;
    $("version").textContent = `squish ${info.version}`;
    const formats = info.formats.map((f) => f.toUpperCase()).join(", ");
    $("limits").textContent = `${formats}, up to ${limitText()} each`;
    $("drop").querySelector(".lead").textContent = `Drop ${info.formats.map((f) => f.toUpperCase()).join(" or ")} images here`;
  })
  .catch(() => {});

// --- adding files ------------------------------------------------------------

function add(file, path, fromWalk) {
  if (fromWalk && (!IMAGE_EXT.test(file.name) || /\.min\.[^.]+$/i.test(file.name) || file.name.startsWith("."))) return;
  const item = {
    file,
    path: path || file.name,
    query: settingsQuery(),
    status: "queued",
    output: null,
    tries: 0,
    row: makeRow(path || file.name),
  };
  items.push(item);
  if (file.size > limits.max_body_bytes) {
    finish(item, "failed", `above the ${limitText()} limit`);
  } else if (file.size === 0) {
    finish(item, "failed", "the file is empty");
  } else {
    queue.push(item);
  }
  render(item);
  $("actions").hidden = false;
  pump();
}

$("pick-files").addEventListener("click", () => $("files").click());
$("pick-folder").addEventListener("click", () => $("folder").click());
$("files").addEventListener("change", (e) => {
  for (const f of e.target.files) add(f, f.name, false);
  e.target.value = "";
});
$("folder").addEventListener("change", (e) => {
  for (const f of e.target.files) add(f, f.webkitRelativePath || f.name, true);
  e.target.value = "";
});

const drop = $("drop");
drop.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    $("files").click();
  }
});
for (const type of ["dragenter", "dragover"]) {
  document.addEventListener(type, (e) => {
    e.preventDefault();
    drop.classList.add("over");
  });
}
document.addEventListener("dragleave", (e) => {
  if (!e.relatedTarget) drop.classList.remove("over");
});
document.addEventListener("drop", (e) => {
  e.preventDefault();
  drop.classList.remove("over");
  const entries = [...e.dataTransfer.items]
    .filter((i) => i.kind === "file")
    .map((i) => (i.webkitGetAsEntry ? i.webkitGetAsEntry() : null));
  if (entries.length && entries.every(Boolean)) {
    for (const entry of entries) walk(entry, "", false);
  } else {
    for (const f of e.dataTransfer.files) add(f, f.name, false);
  }
});

// Dropped folders arrive as entries. Files inside them follow the folder walk rules.
function walk(entry, prefix, inFolder) {
  if (entry.isFile) {
    entry.file((f) => add(f, prefix + f.name, inFolder));
  } else if (entry.isDirectory && !(inFolder && entry.name.startsWith("."))) {
    const reader = entry.createReader();
    const next = () =>
      reader.readEntries((batch) => {
        if (!batch.length) return;
        for (const child of batch) walk(child, `${prefix}${entry.name}/`, true);
        next(); // readEntries returns at most 100 entries per call
      });
    next();
  }
}

// --- the queue ---------------------------------------------------------------

function pump() {
  while (running < IN_FLIGHT && queue.length) {
    const item = queue.shift();
    running++;
    upload(item).finally(() => {
      running--;
      pump();
    });
  }
}

async function upload(item) {
  item.status = "compressing";
  item.tries++;
  render(item);
  let response;
  try {
    response = await fetch(`api/compress?${item.query}`, { method: "POST", body: item.file });
  } catch {
    return finish(item, "failed", "network error, is squish serve still running?");
  }
  if (item.cancelled) return;
  if (response.status === 429 && item.tries < MAX_TRIES) {
    // Back off, then go to the front of the queue so files keep their order.
    item.status = "queued";
    render(item);
    const wait = Math.min(1000 * 2 ** (item.tries - 1), 8000) * (0.5 + Math.random() / 2);
    await new Promise((r) => setTimeout(r, Math.max(wait, retryAfter(response))));
    if (!item.cancelled) queue.unshift(item);
    return;
  }
  if (!response.ok) {
    let error = { error: "failed", message: `HTTP ${response.status}` };
    try {
      error = await response.json();
    } catch {}
    const unsupported = error.error === "unsupported_format" || error.error === "animated_image";
    return finish(item, unsupported ? "unsupported" : "failed", error.message);
  }
  const blob = await response.blob();
  if (item.cancelled) return;
  item.output = blob;
  item.originalSize = Number(response.headers.get("X-Original-Size")) || item.file.size;
  const kept = response.headers.get("X-Kept-Original") === "true";
  finish(item, kept ? "already_optimized" : "compressed");
}

function retryAfter(response) {
  const secs = Number(response.headers.get("Retry-After"));
  return Number.isFinite(secs) ? secs * 1000 : 1000;
}

function finish(item, status, message = "") {
  item.status = status;
  item.message = message;
  render(item);
  renderTotal();
}

// --- rendering ---------------------------------------------------------------

function makeRow(name) {
  const row = $("row").content.firstElementChild.cloneNode(true);
  row.querySelector(".name").textContent = name;
  row.querySelector(".name").title = name;
  $("rows").append(row);
  return row;
}

const LABELS = {
  queued: "Queued",
  compressing: "Compressing…",
  compressed: "Done",
  already_optimized: "Already optimized",
  unsupported: "Unsupported",
  failed: "Failed",
};

function render(item) {
  const row = item.row;
  row.dataset.status = item.status;
  const status = row.querySelector(".status");
  status.textContent = item.message ? `${LABELS[item.status]}: ${item.message}` : LABELS[item.status];
  status.title = status.textContent;
  if (!item.output) {
    row.querySelector(".sizes").textContent = formatBytes(item.file.size);
    return;
  }
  const before = item.originalSize;
  const after = item.output.size;
  row.querySelector(".sizes").textContent = `${formatBytes(before)} → ${formatBytes(after)}`;
  row.querySelector(".saved").textContent = item.status === "compressed" ? `−${percent(before, after)}` : "";
  const url = URL.createObjectURL(item.output);
  item.url = url;
  const thumb = row.querySelector(".thumb");
  thumb.src = url;
  thumb.alt = item.path;
  const link = row.querySelector(".download");
  link.href = url;
  link.download = baseName(item.path);
  link.hidden = false;
}

function renderTotal() {
  const done = items.filter((i) => i.output);
  $("download-all").disabled = done.length === 0;
  if (!done.length) {
    $("total").textContent = "";
    return;
  }
  const before = done.reduce((n, i) => n + i.originalSize, 0);
  const after = done.reduce((n, i) => n + i.output.size, 0);
  const pending = items.filter((i) => i.status === "queued" || i.status === "compressing").length;
  const tail = pending ? `, ${pending} to go` : "";
  $("total").textContent = `Saved ${formatBytes(before - after)}, ${percent(before, after)}${tail}`;
}

// --- downloads ---------------------------------------------------------------

$("download-all").addEventListener("click", async () => {
  const seen = new Set();
  const files = items
    .filter((i) => i.output)
    .map((i) => ({ name: uniqueName(i.path, seen), input: i.output, lastModified: new Date(i.file.lastModified) }));
  // client-zip writes STORE entries, so this is quick (ADR-0008).
  const blob = await downloadZip(files).blob();
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "squish.zip";
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
});

$("clear").addEventListener("click", () => {
  // Files still in flight finish without touching the page.
  for (const item of items) {
    item.cancelled = true;
    if (item.url) URL.revokeObjectURL(item.url);
  }
  items.length = 0;
  queue.length = 0;
  $("rows").replaceChildren();
  $("actions").hidden = true;
  renderTotal();
});

// Two dropped files can share a path. The second becomes "name (2).ext".
function uniqueName(path, seen) {
  let name = path;
  for (let n = 2; seen.has(name.toLowerCase()); n++) {
    const dot = path.lastIndexOf(".");
    name = dot > path.lastIndexOf("/") ? `${path.slice(0, dot)} (${n})${path.slice(dot)}` : `${path} (${n})`;
  }
  seen.add(name.toLowerCase());
  return name;
}

// --- settings ----------------------------------------------------------------

$("quality").addEventListener("change", () => {
  $("fixed-label").hidden = $("quality").value !== "fixed";
});

function settingsQuery() {
  const q = $("quality").value;
  const fixed = Math.max(0, Math.min(100, Math.round(Number($("fixed").value) || 0)));
  const params = new URLSearchParams({ quality: q === "fixed" ? String(fixed) : q, effort: $("effort").value });
  return params.toString();
}

// --- formatting --------------------------------------------------------------

function formatBytes(n) {
  if (!Number.isFinite(n)) return "?";
  if (n < 1e3) return `${n} B`;
  if (n < 1e6) return `${(n / 1e3).toFixed(1)} KB`;
  if (n < 1e9) return `${(n / 1e6).toFixed(1)} MB`;
  return `${(n / 1e9).toFixed(1)} GB`;
}

// The server counts the body limit in MiB (`--max-body 25` is 26214400 bytes).
function limitText() {
  return `${Math.round(limits.max_body_bytes / 1048576)} MB`;
}

function percent(before, after) {
  return before > 0 ? `${Math.round((100 * (before - after)) / before)}%` : "0%";
}

function baseName(path) {
  return path.slice(path.lastIndexOf("/") + 1);
}
