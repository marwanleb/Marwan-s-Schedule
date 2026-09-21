import { startSky } from "./sky.js";
import { openDetail, zoneLabel } from "./detail.js";
import { rangeHHMM, rangeOf, time12 } from "./clock.js";

const invoke = window.__TAURI__.core.invoke;

const CAT_COLOR = {
  work: "#7FD0EC",
  life: "#FFCF95",
  body: "#A8D98F",
  social: "#FF8A76",
};
const DAY_MS = 86400000;
/** How long a ticked item stays on the list before it clears itself. Long
 *  enough to see it happen and undo a mis-tap; short enough that the list does
 *  not silt up with everything you have ever finished. */
const KEEP_DONE_MS = 60 * 60 * 1000;
const ROW_H = 46;
const ROW_H_COLLAPSED = 17;

const state = {
  anchor: null,       // Monday of the shown week, "YYYY-MM-DD"
  week: null,
  items: [],
  session: null,      // running timer, or null
  pinDay: null,       // clicked day — sticky filter
  following: true,    // showing the rolling window, so it moves when the date does
  focus: "schedule",
};

const $ = (id) => document.getElementById(id);

// Handy when poking at the running app from the devtools console.
window.__state = state;

/* ── dates ─────────────────────────────────────────────────────────── */
const iso = (d) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const fromIso = (s) => {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
};
/** The window starts yesterday, so today is always second from the left and
 *  the view rolls forward a day at a time instead of jumping on Mondays. */
const rollingStart = () => {
  const d = new Date();
  d.setDate(d.getDate() - 1);
  return iso(d);
};
const WD = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
const MON = ["JAN","FEB","MAR","APR","MAY","JUN","JUL","AUG","SEP","OCT","NOV","DEC"];
function fmtDur(sec) {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}
function fmtMins(m) {
  if (m % 60 === 0) return `${m / 60}h`;
  return m > 60 ? `${Math.floor(m / 60)}h${m % 60}m` : `${m}m`;
}

/* ── loading ───────────────────────────────────────────────────────── */
async function refresh() {
  // Re-read every time, so a window left open overnight rolls to the new day.
  if (state.following) state.anchor = rollingStart();
  const [week, items, session] = await Promise.all([
    invoke("cmd_get_week", { anchor: state.anchor }),
    invoke("cmd_get_items", { openOnly: false }),
    invoke("cmd_active_session"),
  ]);
  state.week = week;
  state.anchor = week.anchor;
  state.items = items;
  state.session = session;
  render();
  refreshStats();
}

async function refreshStats() {
  const s = await invoke("cmd_stats", { tag: null });
  $("statsLine").textContent = s.phrase
    ? `things take ${s.phrase} than you estimate  ·  ${s.n} timed`
    : s.n === 0
      ? "start a timer on something to learn how long it really takes"
      : `${s.n} timed so far — 5 needed before the estimate figure means anything`;
}

/* ── rendering ─────────────────────────────────────────────────────── */
function render() {
  renderHeader();
  renderDiagnostics();
  renderWeek();
  renderTodos();
  renderTimer();
}

function renderHeader() {
  // Seven days from wherever the window starts. Not a calendar week, so the
  // title is the range itself.
  const first = fromIso(state.anchor);
  const last = new Date(first);
  last.setDate(last.getDate() + 6);
  $("weekTitle").textContent = first.getMonth() === last.getMonth()
    ? `${MON[first.getMonth()]} ${first.getDate()} - ${last.getDate()}`
    : `${MON[first.getMonth()]} ${first.getDate()} - ${MON[last.getMonth()]} ${last.getDate()}`;
  const tz = state.week.viewing_tz || "";
  $("zoneChip").textContent = tz.split("/").pop().replace(/_/g, " ").toUpperCase() || "—";
  $("zoneChip").title = tz;
}

function renderDiagnostics() {
  const box = $("diagnostics");
  const ds = state.week.diagnostics || [];
  box.hidden = ds.length === 0;
  box.innerHTML = "";
  // Never silently drop a rule: if something could not be rendered, say so.
  for (const d of ds) {
    const row = document.createElement("div");
    row.innerHTML = `<span class="tag">NOTE</span>${escapeHtml(d.message)}`;
    box.appendChild(row);
  }
}

/** Hours with nothing on any day of the week collapse, so the useful part of
 *  the day fills the pane. */
function occupiedHours() {
  const used = new Set();
  for (const day of state.week.days) {
    for (const p of day.placements) {
      const s = new Date(p.starts_at);
      const a = s.getHours();
      const b = new Date(p.ends_at);
      // A block that runs up to midnight ends at hour 0 of the next date; read
      // from the clock alone that is "before" it started, and the rows it
      // covers after its first hour were never drawn.
      const endH = b.toDateString() !== s.toDateString() ? 23
        : b.getMinutes() > 0 ? b.getHours() : b.getHours() - 1;
      for (let h = a; h <= Math.max(a, endH); h++) used.add(h);
    }
  }
  return used;
}

function renderWeek() {
  const todayIso = iso(new Date());
  const heads = $("dayHeads");
  heads.innerHTML = "";

  // Matches the hour gutter below, so the headers sit over their columns.
  const spacer = document.createElement("div");
  spacer.className = "headGutter";
  heads.appendChild(spacer);

  // Exactly one column is wide: today's.
  const wideDay = todayIso;

  state.week.days.forEach((day) => {
    const b = document.createElement("button");
    b.className = "dayHead";
    // From the date, not the column: the window starts yesterday, so the first
    // column is Monday only one day in seven.
    const d = fromIso(day.date);
    const dow = (d.getDay() + 6) % 7;
    if (day.date === todayIso) b.classList.add("today");
    if (dow >= 5) b.classList.add("weekend");
    if (state.pinDay === day.date) b.classList.add("sel");
    if (day.date === wideDay) b.classList.add("wide");
    b.innerHTML = `<span class="wd">${WD[dow]}</span><span class="dn">${d.getDate()}</span>`;
    b.onclick = (e) => {
      e.stopPropagation();
      state.pinDay = state.pinDay === day.date ? null : day.date;
      focusPane("todos");
      render();
    };
    heads.appendChild(b);
  });

  const used = occupiedHours();
  // 7am to 10pm by default, stretched to take in anything earlier or later.
  // It used to be exactly 7 to 22, and a block outside that was skipped: a 6am
  // run or an 11:30pm call existed and was never drawn.
  const hours = [];
  for (let h = Math.min(7, ...used); h <= Math.max(22, ...used); h++) hours.push(h);

  // Empty hours give up only as much height as it takes to fit the pane, and
  // never below the floor. All-or-nothing collapsing squeezed a schedule that
  // had room to spare; if the squeeze is not enough, the grid scrolls instead
  // of crushing the day.
  // The grid's allocated box, not clientHeight: this runs before the new rows
  // are appended, so clientHeight would still be describing the old content.
  // getBoundingClientRect is set by the flex layout and is stable either way.
  const PAD_BOTTOM = 16;
  const available = $("grid").clientHeight - PAD_BOTTOM;
  const empties = hours.filter((h) => !used.has(h)).length;
  const deficit = Math.max(0, hours.length * ROW_H - available);
  const give = empties ? Math.min(ROW_H - ROW_H_COLLAPSED, deficit / empties) : 0;
  // Floor, not round: rounding up puts the total back over the edge and the
  // pane scrolls by a few pixels for no reason.
  const emptyH = Math.max(ROW_H_COLLAPSED, Math.floor(ROW_H - give));
  const heightOf = (h) => (used.has(h) ? ROW_H : emptyH);

  const offsets = {};
  let acc = 0;
  for (const h of hours) {
    offsets[h] = acc;
    acc += heightOf(h);
  }

  const grid = $("grid");
  grid.innerHTML = "";
  const inner = document.createElement("div");
  inner.className = "gridInner";

  const gutter = document.createElement("div");
  gutter.className = "hours";
  for (const h of hours) {
    const c = document.createElement("div");
    // Below about two thirds there is no room for "10 AM".
    const small = heightOf(h) < ROW_H * 0.66;
    c.className = "hourCell" + (small ? " collapsed" : "");
    c.style.height = `${heightOf(h)}px`;
    const ampm = h < 12 ? "AM" : "PM";
    const h12 = h % 12 === 0 ? 12 : h % 12;
    c.textContent = small ? `${h12}${ampm[0].toLowerCase()}` : `${h12} ${ampm}`;
    gutter.appendChild(c);
  }
  inner.appendChild(gutter);

  let blocks = 0;
  state.week.days.forEach((day) => {
    const col = document.createElement("div");
    col.className = "dayCol"
      + (day.date === todayIso ? " today" : "")
      + (day.date === wideDay ? " wide" : "");
    col.onclick = (e) => {
      // An existing block owns its own click; so does the composer once open.
      if (e.target.closest(".ev")) return;
      openFormForSlot(hitTest(e.clientX, e.clientY));
    };
    col.style.height = `${acc}px`;
    for (const h of hours) {
      const sp = document.createElement("div");
      sp.className = "spacer";
      sp.style.height = `${heightOf(h)}px`;
      col.appendChild(sp);
    }

    for (const p of day.placements) {
      blocks++;
      const s = new Date(p.starts_at);
      const e = new Date(p.ends_at);
      const startH = s.getHours();
      if (offsets[startH] === undefined) continue;

      const top = offsets[startH] + (s.getMinutes() / 60) * heightOf(startH);
      const mins = Math.max(15, (e - s) / 60000);
      const height = Math.max(18, (mins / 60) * ROW_H);

      const color = CAT_COLOR[p.category] || "#7FD0EC";
      const ev = document.createElement("div");
      ev.className = "ev" + (p.done ? " done" : "");
      ev.style.top = `${top}px`;
      ev.style.height = `${height}px`;
      ev.style.borderLeftColor = color;
      ev.style.background = hexA(color, 0.13);
      ev.style.color = color;

      const zone = p.foreign && p.pinned_tz
        ? `<span class="zone">${escapeHtml(zoneLabel(p.pinned_tz))}</span>`
        : "";
      const rep = p.recurs ? `<span class="rep">&#8635;</span>` : "";
      const place = p.location ? `<span class="place">${escapeHtml(p.location)}</span>` : "";
      ev.innerHTML =
        `<span class="t">${escapeHtml(p.title)}</span>` +
        `<span class="h" style="color:${color}">${rangeOf(p.starts_at, p.ends_at)}${rep}${zone}${place}</span>`;

      ev.onclick = (evt) => {
        evt.stopPropagation();
        // The release that ends a move or resize is also a click; it must not
        // open the popover.
        if (ev.dataset.dragged) return;
        const item = state.items.find((i) => i.id === p.item_id);
        if (item) openDetail(item, day.date, refresh);
      };
      ev.ondblclick = (evt) => {
        evt.stopPropagation();
        toggleTimer(p.item_id, p.id);
      };
      ev.title = p.recurs
        ? "click to edit · drag to move just this week · double-click to time it"
        : "click to edit · drag to move · drag the bottom edge to resize";

      const grip = document.createElement("div");
      grip.className = "evGrip";
      grip.title = "drag to change how long";
      ev.appendChild(grip);

      makeBlockDraggable(ev, grip, p, day.date, hours, heightOf);
      col.appendChild(ev);
    }

    inner.appendChild(col);
  });

  grid.appendChild(inner);

  // The grid scrolls and the header does not, so the header needs the
  // scrollbar's width added to its right padding.
  const sbw = grid.offsetWidth - grid.clientWidth;
  heads.style.paddingRight = `${12 + sbw}px`;

  // One template string, applied to both rows, so the headers and the columns
  // cannot disagree about where a day begins.
  const template = `52px ${state.week.days
    .map((d) => (d.date === wideDay ? "1.5fr" : "1fr"))
    .join(" ")}`;
  heads.style.gridTemplateColumns = template;
  inner.style.gridTemplateColumns = template;

  // Drop targets are measured from the real column rects rather than by
  // dividing the width by seven — the columns are not equal widths.
  state.dropZones = Array.from(inner.querySelectorAll(".dayCol")).map((col, i) => ({
    col,
    date: state.week.days[i].date,
    hours,
    heightOf,
  }));

  $("blockCount").textContent = `${blocks} block${blocks === 1 ? "" : "s"}`;
  placeNowLine();
}

/** A gold line across the week at the current time, while today is on screen.
 *  Placed from the same row geometry as the blocks, and moved by the minute
 *  sweep below rather than by re-rendering the grid. */
function placeNowLine() {
  const zones = state.dropZones || [];
  const today = zones.find((z) => z.date === iso(new Date()));
  const inner = zones[0] && zones[0].col.parentElement;
  const existing = document.getElementById("nowLine");
  const now = new Date();
  // Nothing to show when today is not in view, or the hour is outside the rows.
  if (!today || !inner || !today.hours.includes(now.getHours())) {
    existing?.remove();
    return;
  }
  let y = 0;
  for (const h of today.hours) {
    if (h === now.getHours()) break;
    y += today.heightOf(h);
  }
  y += (now.getMinutes() / 60) * today.heightOf(now.getHours());

  let line = existing;
  if (!line) {
    line = Object.assign(document.createElement("div"), { id: "nowLine", className: "nowLine" });
    line.append(Object.assign(document.createElement("span"), { className: "nowTime" }));
  }
  if (line.parentElement !== inner) inner.appendChild(line);
  line.firstElementChild.textContent = time12(now.getHours(), now.getMinutes());
  // From the first day column -- measured, since today's is wider -- to the
  // inner edge of the scrolling box, past the last column and the grid's
  // padding, so it reaches the side of the pane instead of stopping short.
  // Floored: a fractional pixel of overflow would make the grid scroll sideways.
  const first = zones[0].col;
  const box = inner.parentElement;
  const start = inner.getBoundingClientRect().left + first.offsetLeft;
  const end = box.getBoundingClientRect().left + box.clientLeft + box.clientWidth;
  line.style.top = `${first.offsetTop + y}px`;
  line.style.left = `${first.offsetLeft}px`;
  line.style.width = `${Math.floor(end - start)}px`;
}

/** Moves the line and its clock on the minute, not merely once a minute: a
 *  clock up to 59 seconds behind reads as wrong. */
function tickNowLine() {
  placeNowLine();
  const now = new Date();
  setTimeout(tickNowLine, 60000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 20);
}

/** Buckets are derived from the due date, never stored — otherwise "today"
 *  goes stale the moment tomorrow arrives. */
function bucketOf(item) {
  if (!item.due_at) return "someday";
  const due = new Date(item.due_at);
  const today = new Date();
  today.setHours(23, 59, 59, 999);
  if (due <= today) return "today";
  // "This week" is the next seven days from today, not whichever seven are on
  // screen, so browsing ahead does not reshuffle the list. The same line
  // decides whether a chip shows a weekday or a date.
  return daysAway(due) < 7 ? "week" : "someday";
}

/** Whole calendar days from today to the date's own day; negative if past.
 *  Rounded, because a day with a DST change is 23 or 25 hours long. */
function daysAway(d) {
  const a = new Date();
  a.setHours(0, 0, 0, 0);
  const b = new Date(d);
  b.setHours(0, 0, 0, 0);
  return Math.round((b - a) / 86400000);
}

/** Within a week the weekday is enough and reads fastest. Further out "FRI"
 *  could be any Friday, so it becomes the date and how far away it is. */
function dueLabel(d) {
  const n = daysAway(d);
  if (Math.abs(n) < 7) return WD[(d.getDay() + 6) % 7];
  const date = `${d.getDate()} ${MON[d.getMonth()]}`;
  return n > 0 ? `${date} · ${n}D` : `${date} · ${-n}D AGO`;
}

/** Nearest deadline first. Undated sink to the bottom, and so do ticked
 *  items — they clear themselves within the hour and should not push a live
 *  deadline down the list while they wait. */
function byDate(a, b) {
  if (a.done !== b.done) return a.done ? 1 : -1;
  const ta = a.due_at ? new Date(a.due_at).getTime() : Infinity;
  const tb = b.due_at ? new Date(b.due_at).getTime() : Infinity;
  if (ta !== tb) return ta - tb;
  return a.title.localeCompare(b.title);
}

function renderTodos() {
  const wrap = $("buckets");
  wrap.innerHTML = "";

  // A pinned day filters the list to that day.
  const focusDay = state.pinDay;
  // A finished item lingers for an hour, then clears. Repeating items are
  // unaffected — their tick belongs to one occurrence and lapses on its own.
  const now = Date.now();
  let pool = state.items.filter((i) => {
    if (!i.listed) return false;
    if (!i.done || !i.completed_at) return true;
    return now - new Date(i.completed_at).getTime() < KEEP_DONE_MS;
  });
  if (focusDay) {
    pool = pool.filter((i) => i.due_at && iso(new Date(i.due_at)) === focusDay);
  }

  const bar = $("filterBar");
  if (focusDay) {
    const d = fromIso(focusDay);
    const wd = WD[(d.getDay() + 6) % 7];
    bar.hidden = false;
    $("filterLabel").textContent = `FILTERED · ${wd}`;
    $("filterClear").hidden = false;
  } else {
    bar.hidden = true;
  }

  const groups = { today: [], week: [], someday: [] };
  for (const i of pool) groups[bucketOf(i)].push(i);
  for (const key in groups) groups[key].sort(byDate);

  const labels = { today: "TODAY", week: "THIS WEEK", someday: "SOMEDAY" };
  for (const key of ["today", "week", "someday"]) {
    const list = groups[key];
    const sec = document.createElement("div");
    sec.className = `bucket ${key}`;
    const open = list.filter((i) => !i.done).length;
    sec.innerHTML =
      `<div class="bucketHead"><span class="lbl">${labels[key]}</span>` +
      `<span class="rule"></span><span class="n">${open} open</span></div>`;

    if (list.length === 0) {
      const e = document.createElement("div");
      e.className = "empty";
      e.textContent = focusDay ? "nothing here" : "—";
      sec.appendChild(e);
    }

    for (const item of list) sec.appendChild(taskRow(item));
    wrap.appendChild(sec);
  }

  const open = state.items.filter((i) => i.listed && !i.done).length;
  $("openCount").textContent = `${open} open`;
}

function taskRow(item) {
  const color = CAT_COLOR[item.category] || "#59637a";
  const row = document.createElement("div");
  row.className = "task" + (item.done ? " done" : "");
  row.style.borderLeftColor = item.done ? "#2a3140" : color;

  const box = document.createElement("button");
  box.className = "box";
  box.innerHTML = "<i></i>";
  box.style.borderColor = item.done ? color : "";
  box.style.background = item.done ? color : "";
  box.title = item.recurs ? "tick off this week" : "tick off";
  box.onclick = async (e) => {
    e.stopPropagation();
    await invoke("cmd_set_done", { id: item.id, done: !item.done, on: null });
    await refresh();
  };

  const txt = document.createElement("span");
  txt.className = "txt";
  txt.textContent = item.title;

  const chips = document.createElement("span");
  chips.className = "chips";
  if (item.estimate_min) {
    const over = item.spent_sec > item.estimate_min * 60;
    chips.appendChild(chip(`~${fmtMins(item.estimate_min)}`, over ? "#FF8A76" : "#59637a"));
  }
  if (item.due_at) {
    const d = new Date(item.due_at);
    chips.appendChild(chip(dueLabel(d), color));
  }
  if (item.recurs) chips.appendChild(chip("↻", color));

  const play = document.createElement("button");
  play.className = "play";
  play.style.color = color;
  const running = state.session && state.session.item_id === item.id;
  play.textContent = running ? "■" : "▶";
  play.title = running ? "stop the timer" : "start a timer";
  play.onclick = (e) => {
    e.stopPropagation();
    toggleTimer(item.id, null);
  };
  chips.appendChild(play);

  txt.onclick = () => openDetail(item, null, refresh);
  txt.style.cursor = "pointer";
  txt.title = "click to edit";

  row.append(box, txt, chips);
  makeDraggable(row, item);
  return row;
}

function chip(text, color) {
  const s = document.createElement("span");
  s.className = "tagChip";
  s.style.color = color;
  s.textContent = text;
  return s;
}

/* ── timer ─────────────────────────────────────────────────────────── */
async function toggleTimer(itemId, placementId) {
  if (state.session && state.session.item_id === itemId) {
    await invoke("cmd_timer_stop", { sessionId: state.session.id });
  } else {
    // Starting a second timer stops the first; the backend reports which.
    await invoke("cmd_timer_start", { itemId, placementId });
  }
  await refresh();
}

$("timerStop").onclick = async (e) => {
  e.stopPropagation();
  if (state.session) await invoke("cmd_timer_stop", { sessionId: state.session.id });
  await refresh();
};

function renderTimer() {
  const bar = $("timerBar");
  if (!state.session) {
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  $("timerTitle").textContent = state.session.title;
  const est = state.session.estimate_min;
  $("timerEstimate").textContent = est ? `est ${fmtMins(est)}` : "";
  tickTimer();
}

function tickTimer() {
  if (!state.session) return;
  const banked = state.session.spent_sec || 0;
  const live = (Date.now() - new Date(state.session.started_at).getTime()) / 1000;
  const total = banked + Math.max(0, live);
  $("timerElapsed").textContent = fmtDur(total);
  const est = state.session.estimate_min;
  $("timerEstimate").classList.toggle("over", !!est && total > est * 60);
}
// The clock is rendered locally; nothing polls the backend for it.
setInterval(tickTimer, 1000);

// Ticked items clear on their own, so a window left open has to notice the
// hour passing. Re-renders only when something has actually aged out.
setInterval(() => {
  if (!state.week) return;
  // Midnight passed with the window open: roll it on by a day.
  if (state.following && state.anchor !== rollingStart()) {
    refresh();
    return;
  }
  const now = Date.now();
  const stale = state.items.some(
    (i) => i.listed && i.done && i.completed_at &&
      now - new Date(i.completed_at).getTime() >= KEEP_DONE_MS
  );
  if (stale) render();
}, 60_000);

/* ── input ─────────────────────────────────────────────────────────── */
$("addForm").onsubmit = async (e) => {
  e.preventDefault();
  const input = $("addInput");
  const text = input.value.trim();
  if (!text) return;

  if (/^\/?(help|\?|h)$/i.test(text)) {
    await showHelp();
    input.value = "";
    return;
  }
  const res = await invoke("cmd_add", { text });
  input.value = "";
  await refresh();
  showEcho(res);
};

/* ── the field form ─────────────────────────────────────────────────── */
/* Fields are rendered into a line of the ordinary grammar by core, and that
   line goes through cmd_add like anything typed. The form has no privileged
   path into the store, and it shows you the sentence it is building — which is
   how you stop needing it. */

const DAY_CODES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const form = {
  kind: "task",
  days: new Set(),
  monthly: false,
};

function buildDayButtons() {
  const wrap = $("ffDays");
  for (const code of DAY_CODES) {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = code[0].toUpperCase();
    b.title = code;
    b.onclick = () => {
      form.days.has(code) ? form.days.delete(code) : form.days.add(code);
      b.classList.toggle("on", form.days.has(code));
      if (form.monthly) {
        form.monthly = false;
        wrap.querySelector(".ffMonthly")?.classList.remove("on");
      }
      refreshPreview();
    };
    wrap.appendChild(b);
  }

  // Monthly and weekdays are two different cadences; picking one clears the
  // other rather than composing a line that means neither.
  const m = document.createElement("button");
  m.type = "button";
  m.className = "ffMonthly";
  m.textContent = "MONTHLY";
  m.title = "on the date's day each month";
  m.onclick = () => {
    form.monthly = !form.monthly;
    m.classList.toggle("on", form.monthly);
    if (form.monthly) {
      form.days.clear();
      for (const b of wrap.children) if (b !== m) b.classList.remove("on");
    }
    refreshPreview();
  };
  wrap.appendChild(m);
}

/** "90m", "2h", "1.5h" or a bare number of minutes. Anything else is nothing,
 *  rather than a guess. */
function readEstimate(raw) {
  const s = raw.trim().toLowerCase().replace(/^~/, "");
  if (!s) return null;
  const m = s.match(/^(\d+(?:\.\d+)?)\s*(h|hr|hrs|hour|hours|m|min|mins|minute|minutes)?$/);
  if (!m) return null;
  const n = parseFloat(m[1]);
  if (!isFinite(n) || n <= 0) return null;
  const mins = /^h/.test(m[2] || "m") ? Math.round(n * 60) : Math.round(n);
  return mins > 0 ? mins : null;
}

function readFields() {
  const time = $("ffTime").value || null;
  const end = $("ffEnd").value || null;
  return {
    title: $("ffTitle").value.trim(),
    priority: $("ffPriority").checked,
    repeat: form.monthly ? [] : DAY_CODES.filter((d) => form.days.has(d)),
    monthly: form.monthly,
    kind: form.kind,
    date: $("ffDate").value || null,
    at: time,
    // An end without a start is not a range; the parser would read it as a
    // lone time and the round trip would not hold.
    span_end: time ? end : null,
    estimate_min: readEstimate($("ffEst").value),
    tag: $("ffTag").value.trim() || null,
    place: $("ffPlace").value.trim() || null,
  };
}

let previewLine = "";
async function refreshPreview() {
  const f = readFields();
  // A repeat has no single date. Say so by greying the field rather than
  // letting someone fill in a date the grammar will drop.
  // A monthly repeat keeps it: the date says which day of the month.
  const repeating = f.repeat.length > 0;
  $("ffDate").disabled = repeating;
  $("ffDate").title = repeating ? "a repeat has no single date"
    : f.monthly ? "the day of the month it repeats on" : "";

  if (!f.title) {
    previewLine = "";
    const box = $("ffPreview");
    box.textContent = "give it a title";
    box.classList.add("empty");
    return;
  }
  try {
    previewLine = await invoke("cmd_compose", { fields: f });
    const box = $("ffPreview");
    box.textContent = previewLine;
    box.classList.remove("empty");
  } catch (err) {
    showError(String(err));
  }
}

async function submitFields() {
  if (!previewLine) return;
  const res = await invoke("cmd_add", { text: previewLine });
  for (const id of ["ffTitle", "ffDate", "ffTime", "ffEnd", "ffEst", "ffTag", "ffPlace"]) {
    $(id).value = "";
  }
  $("ffPriority").checked = false;
  form.days.clear();
  form.monthly = false;
  for (const b of $("ffDays").children) b.classList.remove("on");
  await refreshPreview();
  await refresh();
  showEcho(res);
  $("ffTitle").focus();
}

function initForm() {
  buildDayButtons();
  const panel = $("addFields");
  $("formToggle").onclick = () => {
    panel.hidden = !panel.hidden;
    $("formToggle").classList.toggle("open", !panel.hidden);
    if (!panel.hidden) {
      $("ffTitle").value = $("addInput").value.trim();
      refreshPreview();
      $("ffTitle").focus();
    }
  };

  for (const id of ["ffTitle", "ffDate", "ffTime", "ffEnd", "ffEst", "ffTag", "ffPlace"]) {
    $(id).oninput = refreshPreview;
    $(id).onkeydown = (e) => {
      e.stopPropagation();
      if (e.key === "Enter") {
        e.preventDefault();
        submitFields();
      }
    };
  }
  $("ffPriority").onchange = refreshPreview;
  for (const b of $("ffKind").children) {
    b.onclick = () => {
      form.kind = b.dataset.kind;
      for (const other of $("ffKind").children) other.classList.toggle("on", other === b);
      refreshPreview();
    };
  }
  $("ffAdd").onclick = submitFields;
  refreshPreview();
}
initForm();

let echoTimer = null;
/** Say what was understood. A line the parser did not recognise is saved whole
 *  — which is correct, but silent, and silence is how a typo goes unnoticed. */
function showEcho(res) {
  const box = $("addEcho");
  clearTimeout(echoTimer);

  if (!res.consumed.length) {
    box.className = "addEcho plain";
    box.innerHTML =
      `saved as plain text — nothing recognised in <b>${escapeHtml(res.item.title)}</b>. type <b>help</b> for the syntax`;
  } else {
    const bits = [];
    if (res.byday.length) bits.push(`repeats ${res.byday.join(" ")}`);
    if (res.span) bits.push(rangeHHMM(res.span[0], res.span[1]));
    if (res.item.due_at && !res.byday.length) {
      bits.push(`due ${new Date(res.item.due_at).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}`);
    }
    if (res.item.estimate_min) bits.push(`~${fmtMins(res.item.estimate_min)}`);
    if (res.location) bits.push(res.location);
    box.className = "addEcho";
    box.innerHTML = `<b>${escapeHtml(res.item.title)}</b> · ${escapeHtml(bits.join(" · "))}`;
  }

  box.hidden = false;
  echoTimer = setTimeout(() => (box.hidden = true), 6000);
}

async function showHelp() {
  $("helpBody").textContent = await invoke("cmd_help");
  $("helpOverlay").hidden = false;
}
$("detailClose").onclick = () => ($("detailOverlay").hidden = true);
$("detailOverlay").onclick = (e) => {
  if (e.target === $("detailOverlay")) $("detailOverlay").hidden = true;
};
$("helpClose").onclick = () => ($("helpOverlay").hidden = true);
$("helpOverlay").onclick = (e) => {
  if (e.target === $("helpOverlay")) $("helpOverlay").hidden = true;
};

function focusPane(which) {
  state.focus = which;
  $("schedulePane").classList.toggle("focused", which === "schedule");
  $("todoPane").classList.toggle("focused", which === "todos");
}
$("schedulePane").onclick = () => { $("weekPicker").hidden = true; focusPane("schedule"); };
$("todoPane").onclick = () => { $("weekPicker").hidden = true; focusPane("todos"); };

function shiftWeek(delta) {
  const d = fromIso(state.anchor);
  d.setDate(d.getDate() + delta * 7);
  state.anchor = iso(d);
  // Browsing away stops the window following the date; coming back resumes it.
  state.following = state.anchor === rollingStart();
  refresh();
}
$("filterClear").onclick = (e) => {
  e.stopPropagation();
  state.pinDay = null;
  render();
};
$("prevWeek").onclick = (e) => { e.stopPropagation(); shiftWeek(-1); };
$("nextWeek").onclick = (e) => { e.stopPropagation(); shiftWeek(1); };
/** Clicking through a week at a time to reach something three months out is
 *  no way to plan; the picker jumps straight there. */
function renderPicker() {
  const rows = $("pickRows");
  rows.innerHTML = "";
  // Rows step seven days from the rolling window, so the first one is "now".
  const base = fromIso(rollingStart());
  const shown = fromIso(state.anchor);

  for (let off = -4; off <= 28; off++) {
    const start = new Date(base);
    start.setDate(start.getDate() + off * 7);
    const end = new Date(start);
    end.setDate(end.getDate() + 6);

    const b = document.createElement("button");
    b.className = "pickRow";
    if (off === 0) b.classList.add("now");
    if (iso(start) === iso(shown)) b.classList.add("sel");

    const fmt = (d) => `${MON[d.getMonth()]} ${d.getDate()}`;
    const rel =
      off === 0 ? "THIS WEEK"
      : off === 1 ? "NEXT WEEK"
      : off === -1 ? "LAST WEEK"
      : off > 0 ? `+${off} WKS` : `${off} WKS`;

    b.innerHTML =
      `<span class="range">${fmt(start)} – ${fmt(end)}</span><span class="rel">${rel}</span>`;
    b.onclick = (e) => {
      e.stopPropagation();
      state.anchor = iso(start);
      state.following = off === 0;
      state.pinDay = null;
      $("weekPicker").hidden = true;
      refresh();
    };
    rows.appendChild(b);
  }
}

$("today").onclick = (e) => {
  e.stopPropagation();
  const picker = $("weekPicker");
  if (picker.hidden) {
    renderPicker();
    picker.hidden = false;
    // Open on the week being shown, not scrolled to the top.
    const sel = picker.querySelector(".pickRow.sel");
    if (sel) sel.scrollIntoView({ block: "center" });
  } else {
    picker.hidden = true;
  }
};

$("hide").onclick = (e) => { e.stopPropagation(); invoke("cmd_hide"); };

// Row heights are fitted to the pane, so a resize has to re-fit them.
// Debounced, because a drag fires this continuously.
let resizeTimer = null;
window.addEventListener("resize", () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (state.week) render();
  }, 120);
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (!$("helpOverlay").hidden) { $("helpOverlay").hidden = true; return; }
    if (!$("detailOverlay").hidden) { $("detailOverlay").hidden = true; return; }
    if (!$("weekPicker").hidden) { $("weekPicker").hidden = true; return; }
    if (document.activeElement === $("addInput")) { $("addInput").blur(); return; }
    invoke("cmd_hide");
  }
  if (e.key === "/" && document.activeElement !== $("addInput")) {
    e.preventDefault();
    $("addInput").focus();
  }
});

/* ── drag a task onto the week ──────────────────────────────────────── */
/** Dropping creates a placement for the SAME item, never a copy. Spec 3.1. */
function makeDraggable(row, item) {
  row.style.touchAction = "none";
  row.addEventListener("pointerdown", (e) => {
    // The checkbox and the timer control own their own clicks.
    if (e.target.closest(".box, .play")) return;

    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    const ghost = $("dragGhost");

    const move = (ev) => {
      if (!dragging) {
        // A few pixels of slop, so a click stays a click.
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < 5) return;
        dragging = true;
        ghost.hidden = false;
        ghost.textContent = item.title;
        ghost.style.borderColor = CAT_COLOR[item.category] || "#59637a";
      }
      ghost.style.left = `${ev.clientX}px`;
      ghost.style.top = `${ev.clientY}px`;
      highlight(hitTest(ev.clientX, ev.clientY));
    };

    const up = async (ev) => {
      try { row.releasePointerCapture?.(e.pointerId); } catch { /* never captured */ }
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      ghost.hidden = true;
      highlight(null);
      if (!dragging) return;

      const hit = hitTest(ev.clientX, ev.clientY);
      if (!hit) return;
      const starts = new Date(`${hit.date}T${String(hit.hour).padStart(2, "0")}:00:00`);
      const mins = item.estimate_min && item.estimate_min <= 480 ? item.estimate_min : 60;
      const ends = new Date(starts.getTime() + mins * 60000);
      try {
        await invoke("cmd_add_placement", {
          itemId: item.id,
          startsAt: rfc(starts),
          endsAt: rfc(ends),
        });
        await refresh();
      } catch (err) {
        showError(String(err));
      }
    };

    // Listeners first: setPointerCapture throws for a pointer the browser is
    // not tracking, and if that ran first the drag would never arm.
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    try {
      row.setPointerCapture?.(e.pointerId);
    } catch {
      // Capture is an optimisation; the window listeners do the real work.
    }
  });
}

/* ── moving and resizing a block ────────────────────────────────────── */

/** Round to the nearest quarter hour. Finer than that is noise on a grid this
 *  size, and coarser loses the 45-minute meeting. */
const SNAP_MIN = 15;
const MIN_BLOCK_MIN = 15;
const snap = (mins) => Math.round(mins / SNAP_MIN) * SNAP_MIN;

/** Minutes from midnight at a y position inside a column, from the real row
 *  geometry — rows are not equal heights once empty ones collapse. */
function minutesAt(y, colTop, hours, heightOf) {
  let acc = colTop;
  for (const h of hours) {
    const height = heightOf(h);
    if (y < acc + height) return h * 60 + ((y - acc) / height) * 60;
    acc += height;
  }
  const last = hours[hours.length - 1];
  return last * 60 + 59;
}

function makeBlockDraggable(ev, grip, p, date, hours, heightOf) {
  ev.style.touchAction = "none";

  ev.addEventListener("pointerdown", (e) => {
    const resizing = e.target === grip;
    const col = ev.parentElement;
    const startY = e.clientY;
    const startX = e.clientX;
    const s = new Date(p.starts_at);
    const eAt = new Date(p.ends_at);
    const lengthMin = Math.max(MIN_BLOCK_MIN, (eAt - s) / 60000);
    const startMin = s.getHours() * 60 + s.getMinutes();
    let dragging = false;
    let latest = null;
    // Where the block sat when picked up, and where the pointer last was.
    const startRect = ev.getBoundingClientRect();
    let lastX = startX;
    let lastY = startY;

    const move = (mv) => {
      if (!dragging) {
        if (Math.hypot(mv.clientX - startX, mv.clientY - startY) < 4) return;
        dragging = true;
        ev.classList.add("dragging");
      }
      lastX = mv.clientX;
      lastY = mv.clientY;

      if (resizing) {
        // The bottom edge follows the pointer exactly; the length snaps on
        // release, the same way a move does.
        ev.style.height = `${Math.max(18, startRect.height + (mv.clientY - startY))}px`;
        return;
      }
      // Follow the pointer exactly, across days as well as hours. Nothing
      // snaps until release; the column it will land in is highlighted.
      ev.style.transform = `translate(${mv.clientX - startX}px, ${mv.clientY - startY}px)`;
      highlight(hitTest(mv.clientX, mv.clientY));
    };

    const up = async (mv) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      ev.classList.remove("dragging");
      if (dragging) {
        // Set before the first await: the browser dispatches the click that
        // follows this pointerup before any timer or network reply can run.
        ev.dataset.dragged = "1";
        setTimeout(() => delete ev.dataset.dragged, 0);
      }
      if (!dragging) return;
      lastX = mv.clientX ?? lastX;
      lastY = mv.clientY ?? lastY;

      if (resizing) {
        // Where the bottom edge was let go decides the end, snapped now rather
        // than while pulling.
        const colTop = col.getBoundingClientRect().top;
        const bottomEdge = startRect.bottom + (lastY - startY);
        const end = Math.max(startMin + MIN_BLOCK_MIN, snap(minutesAt(bottomEdge, colTop, hours, heightOf)));
        latest = { startMin, endMin: Math.min(end, 24 * 60), date };
      } else {
        highlight(null);
        const hit = hitTest(lastX, lastY);
        if (!hit) {
          // Let go outside every column: put it back rather than guess a day.
          ev.style.transform = "";
          return;
        }
        // The block's own top edge decides the time, so the grab point within
        // it is kept; the column under the pointer decides the day.
        const colTop = hit.col.getBoundingClientRect().top;
        const topEdge = startRect.top + (lastY - startY);
        let top = snap(minutesAt(topEdge, colTop, hours, heightOf));
        top = Math.max(0, Math.min(top, 24 * 60 - lengthMin));
        latest = { startMin: top, endMin: top + lengthMin, date: hit.date };
      }
      if (!latest) return;

      const hhmm = (m) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
      const starts = rfc(new Date(`${latest.date}T${hhmm(latest.startMin)}:00`));
      // Up to midnight means the midnight after, on the next date, not 11:59pm.
      const nextDay = fromIso(latest.date);
      nextDay.setDate(nextDay.getDate() + 1);
      const ends = latest.endMin >= 24 * 60
        ? rfc(new Date(`${iso(nextDay)}T00:00:00`))
        : rfc(new Date(`${latest.date}T${hhmm(latest.endMin)}:00`));

      try {
        if (p.recurs) {
          // A recurring occurrence is generated, not stored: moving it means
          // cancelling that date and placing a real block at the new time.
          await invoke("cmd_move_occurrence", {
            itemId: p.item_id,
            date,
            startsAt: starts,
            endsAt: ends,
          });
        } else {
          await invoke("cmd_move_placement", { id: p.id, startsAt: starts, endsAt: ends });
        }
        await refresh();
      } catch (err) {
        showError(String(err));
        await refresh();
      }
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    try {
      ev.setPointerCapture?.(e.pointerId);
    } catch {
      // Capture is an optimisation; the window listeners do the real work.
    }
    e.stopPropagation();
  });
}

/** Which day column and hour the cursor is over, by real geometry. */
function hitTest(x, y) {
  for (const z of state.dropZones || []) {
    const r = z.col.getBoundingClientRect();
    if (x < r.left || x > r.right || y < r.top || y > r.bottom) continue;
    let acc = r.top;
    for (const h of z.hours) {
      const height = z.heightOf(h);
      if (y >= acc && y < acc + height) {
        return { date: z.date, hour: h, col: z.col };
      }
      acc += height;
    }
    return { date: z.date, hour: z.hours[z.hours.length - 1], col: z.col };
  }
  return null;
}

let lastHighlight = null;
function highlight(hit) {
  if (lastHighlight && lastHighlight !== hit?.col) lastHighlight.classList.remove("dropTarget");
  if (hit) hit.col.classList.add("dropTarget");
  lastHighlight = hit?.col || null;
}

/* ── click an empty hour to fill the form in ─────────────────────────── */
/** Opens the same fields the toggle does, with the slot's date and time
 *  already in them. One way to add something, wherever you start from. */
function openFormForSlot(hit) {
  if (!hit) return;
  $("addFields").hidden = false;
  $("formToggle").classList.add("open");
  $("ffDate").value = hit.date;
  $("ffTime").value = `${String(hit.hour).padStart(2, "0")}:00`;
  // Put on the week, so it is a block. The form still lets you say otherwise.
  form.kind = "block";
  for (const b of $("ffKind").children) b.classList.toggle("on", b.dataset.kind === "block");
  refreshPreview();
  $("ffTitle").focus();
}

function rfc(d) {
  const pad = (n) => String(n).padStart(2, "0");
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? "+" : "-";
  const a = Math.abs(off);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:00${sign}${pad(Math.floor(a / 60))}:${pad(a % 60)}`;
}

function showError(msg) {
  const box = $("diagnostics");
  box.hidden = false;
  box.innerHTML = `<div><span class="tag">ERROR</span>${escapeHtml(msg)}</div>`;
}

/* ── helpers ───────────────────────────────────────────────────────── */
function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}
function hexA(hex, a) {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
}

/* ── noticing writes from the bot and the CLI ───────────────────────── */
// The window loaded its data once and then never looked again, so anything
// sent to the Telegram bot sat invisible until something else forced a
// redraw. There is no watcher by design; SQLite's data_version changes when
// another connection writes, which is cheap to ask.
let seenVersion = null;

async function refreshIfChanged() {
  try {
    const v = await invoke("cmd_data_version");
    if (seenVersion !== null && v !== seenVersion) {
      seenVersion = v;
      await refresh();
      return;
    }
    seenVersion = v;
  } catch {
    // A failed check is not worth surfacing; the next one will do.
  }
}

setInterval(() => {
  if (!document.hidden) refreshIfChanged();
}, 5000);

// And immediately on being summoned, so it is never stale when you look at it.
const tauriEvents = window.__TAURI__?.event;
if (tauriEvents) {
  tauriEvents.listen("window:shown", () => refresh());
}
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) refreshIfChanged();
});

/* ── go ────────────────────────────────────────────────────────────── */
state.anchor = rollingStart();
startSky(document.getElementById("sky"));
tickNowLine();
refresh().catch((e) => {
  document.body.insertAdjacentHTML(
    "afterbegin",
    `<div id="diagnostics"><span class="tag">ERROR</span>${escapeHtml(String(e))}</div>`
  );
});
