/**
 * Deleting, from the list row or the detail sheet. There is no "are you
 * sure?": the item goes at once and a toast offers it back for a few seconds.
 * A confirm dialog is asked on every delete to catch the rare wrong one;
 * an undo costs nothing until it is needed.
 */

const invoke = window.__TAURI__.core.invoke;

/** Long enough to notice the wrong row went; short enough not to linger. */
const UNDO_MS = 6000;

let hideTimer = null;
let lastId = null;
let afterUndo = null;

/** Delete `item`, then offer it back. `onChanged` re-reads the store. */
export async function deleteWithUndo(item, onChanged) {
  await invoke("cmd_delete", { id: item.id });
  await onChanged();

  const bar = document.getElementById("undoBar");
  document.getElementById("undoTitle").textContent = item.recurs
    ? `deleted ${item.title}, every repeat`
    : `deleted ${item.title}`;
  lastId = item.id;
  afterUndo = onChanged;
  bar.hidden = false;
  clearTimeout(hideTimer);
  hideTimer = setTimeout(dismiss, UNDO_MS);
}

async function undo() {
  if (!lastId) return;
  const id = lastId;
  dismiss();
  await invoke("cmd_restore", { id });
  if (afterUndo) await afterUndo();
}

function dismiss() {
  clearTimeout(hideTimer);
  document.getElementById("undoBar").hidden = true;
  lastId = null;
}

document.getElementById("undoButton").onclick = undo;

// Cmd/Ctrl+Z while the toast is up, unless a text field wants it for itself.
document.addEventListener("keydown", (e) => {
  if (!lastId || e.key.toLowerCase() !== "z" || !(e.metaKey || e.ctrlKey) || e.shiftKey) return;
  const t = e.target;
  if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) return;
  e.preventDefault();
  undo();
});
