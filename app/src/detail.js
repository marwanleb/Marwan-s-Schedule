import { rangeHHMM } from "./clock.js";
/**
 * The item detail sheet: everything you can change about one item without
 * opening a terminal — estimate, whether it shows in the list, whether it
 * repeats and on which days, which zone it is fixed to, and cancelling or
 * deleting a single occurrence.
 */

const invoke = window.__TAURI__.core.invoke;
const WD = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const WD_LABEL = ["M", "T", "W", "T", "F", "S", "S"];

const el = (tag, cls, text) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
};

/** Short display label for a zone; the full IANA name is what gets stored. */
export const zoneLabel = (z) =>
  !z ? "SYSTEM" : z.split("/").pop().replace(/_/g, " ").toUpperCase();

export async function openDetail(item, occurrenceDate, onChanged) {
  const overlay = document.getElementById("detailOverlay");
  const body = document.getElementById("detailBody");
  overlay.hidden = false;
  body.innerHTML = "";

  const [rule, zones] = await Promise.all([
    invoke("cmd_get_recurrence", { itemId: item.id }),
    invoke("cmd_zones"),
  ]);

  const refresh = async () => {
    await onChanged();
    overlay.hidden = true;
  };

  /* ── heading ─────────────────────────────────────────────────────── */
  const eyebrow = [rule ? "REPEATING" : "ONCE", (item.category || "no category").toUpperCase()]
    .join(" · ");
  body.append(el("div", "eyebrow", eyebrow));
  body.append(el("h2", "detailTitle", item.title));

  const when = rule
    ? `${rangeHHMM(rule.start_time, rule.end_time)} · ${rule.byday.map((d) => d.toUpperCase()).join(" ")}`
    : item.due_at
      ? `due ${new Date(item.due_at).toLocaleString([], { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}`
      : "no date, no time";
  body.append(el("div", "detailWhen", when + (item.location ? `  ·  ${item.location}` : "")));

  /* ── timer ───────────────────────────────────────────────────────── */
  const timer = el("button", "wide");
  timer.textContent = "START TIMER";
  timer.onclick = async () => {
    await invoke("cmd_timer_start", { itemId: item.id, placementId: null });
    await refresh();
  };
  body.append(timer);

  /* ── estimate ────────────────────────────────────────────────────── */
  body.append(field("ESTIMATE", stepper(
    item.estimate_min,
    (v) => (v ? `${v >= 60 ? `${Math.floor(v / 60)}h${v % 60 ? v % 60 + "m" : ""}` : v + "m"}` : "none"),
    async (next) => {
      await invoke("cmd_set_estimate", { id: item.id, minutes: next });
      await onChanged();
      openDetail({ ...item, estimate_min: next }, occurrenceDate, onChanged);
    },
    30
  )));

  /* ── show in to-do ───────────────────────────────────────────────── */
  const listedBtn = el("button", "toggle" + (item.listed ? " on" : ""));
  listedBtn.textContent = item.listed ? "YES" : "NO";
  listedBtn.onclick = async () => {
    await invoke("cmd_set_listed", { id: item.id, listed: !item.listed });
    await onChanged();
    openDetail({ ...item, listed: !item.listed }, occurrenceDate, onChanged);
  };
  body.append(field("SHOW IN TO DO", listedBtn));

  /* ── time zone ───────────────────────────────────────────────────── */
  if (rule) {
    const chips = el("div", "chipRow");
    const current = rule.tz;
    const options = [null, ...zones];
    for (const z of options) {
      const b = el("button", "zchip" + ((z || null) === (current || null) ? " on" : ""));
      b.textContent = zoneLabel(z);
      b.title = z || "follows this machine as you travel";
      b.onclick = async () => {
        try {
          await invoke("cmd_set_recurrence_tz", { itemId: item.id, tz: z });
          await onChanged();
          openDetail(item, occurrenceDate, onChanged);
        } catch (e) {
          alertLine(body, String(e));
        }
      };
      chips.append(b);
    }
    const wrap = el("div", "stack");
    wrap.append(chips);
    wrap.append(el("div", "hint", current
      ? `Fixed: stays on ${zoneLabel(current)}'s clock wherever you are.`
      : "System: follows this machine as you travel."));
    body.append(field("TIME ZONE", wrap, true));
  }

  /* ── repeat ──────────────────────────────────────────────────────── */
  const repeatBox = el("div", "stack");
  // Where a new repeat is anchored: the occurrence clicked, else the item's own
  // date, else today.
  const anchorDate = occurrenceDate
    ? new Date(occurrenceDate + "T00:00:00")
    : item.due_at ? new Date(item.due_at) : new Date();
  const setRule = (byday, monthday, start = "09:00", end = "10:00") =>
    invoke("cmd_set_recurrence", { itemId: item.id, byday, startTime: start, endTime: end, monthday });
  const toggled = async () => {
    await onChanged();
    openDetail({ ...item, recurs: !rule }, occurrenceDate, onChanged);
  };

  if (rule) {
    const stop = el("button", "wide danger");
    stop.textContent = "STOP REPEATING";
    stop.onclick = async () => { await setRule([], null); await toggled(); };
    repeatBox.append(stop);
  } else {
    const choice = el("div", "repeatChoice");
    const weekly = el("button", "wide");
    weekly.textContent = "REPEAT WEEKLY";
    weekly.onclick = async () => {
      await setRule([WD[(anchorDate.getDay() + 6) % 7]], null);
      await toggled();
    };
    const monthly = el("button", "wide");
    monthly.textContent = "REPEAT MONTHLY";
    monthly.title = `on the ${ordinal(anchorDate.getDate())} of each month`;
    monthly.onclick = async () => {
      await setRule([], anchorDate.getDate());
      await toggled();
    };
    choice.append(weekly, monthly);
    repeatBox.append(choice);
  }

  if (rule) {
    if (rule.monthday) {
      // A monthly rule has no weekdays to toggle; say which day it lands on.
      repeatBox.append(el("div", "hint", `MONTHLY ON THE ${ordinal(rule.monthday).toUpperCase()}` +
        (rule.monthday > 28 ? " — THE LAST DAY IN SHORTER MONTHS" : "")));
    } else {
      const days = el("div", "dayRow");
      WD.forEach((code, i) => {
        const b = el("button", "dchip" + (rule.byday.includes(code) ? " on" : ""));
        b.textContent = WD_LABEL[i];
        b.onclick = async () => {
          const next = rule.byday.includes(code)
            ? rule.byday.filter((d) => d !== code)
            : [...rule.byday, code];
          await invoke("cmd_set_recurrence", {
            itemId: item.id, byday: next,
            startTime: rule.start_time, endTime: rule.end_time, monthday: null,
          });
          await onChanged();
          openDetail(item, occurrenceDate, onChanged);
        };
        days.append(b);
      });
      repeatBox.append(days);
    }

    repeatBox.append(timeEditor(rule, async (start, end) => {
      await invoke("cmd_set_recurrence", {
        itemId: item.id, byday: rule.byday, startTime: start, endTime: end,
        monthday: rule.monthday ?? null,
      });
      await onChanged();
      openDetail(item, occurrenceDate, onChanged);
    }));

    /* except dates */
    const ex = el("div", "stack");
    ex.append(el("div", "hint", rule.except_on.length
      ? "EXCEPT ON THESE DATES"
      : "EXCEPT ON THESE DATES — NONE YET"));
    if (rule.except_on.length) {
      const row = el("div", "chipRow");
      for (const date of rule.except_on) {
        const b = el("button", "xchip");
        b.textContent = `${date} ×`;
        b.title = "put this occurrence back";
        b.onclick = async () => {
          await invoke("cmd_remove_except", { itemId: item.id, date });
          await onChanged();
          openDetail(item, occurrenceDate, onChanged);
        };
        row.append(b);
      }
      ex.append(row);
    }
    repeatBox.append(ex);
  }
  body.append(field("REPEATS", repeatBox, true));

  /* ── this occurrence / destructive ───────────────────────────────── */
  const acts = el("div", "stack");
  if (rule && occurrenceDate) {
    const cancel = el("button", "wide");
    cancel.textContent = `CANCEL JUST ${occurrenceDate}`;
    cancel.title = "removes this one date, leaving the series alone";
    cancel.onclick = async () => {
      await invoke("cmd_except", { id: item.id, date: occurrenceDate });
      await refresh();
    };
    acts.append(cancel);
  }
  const del = el("button", "wide danger");
  del.textContent = rule ? "DELETE WHOLE SERIES" : "DELETE";
  del.onclick = async () => {
    await invoke("cmd_delete", { id: item.id });
    await refresh();
  };
  acts.append(del);
  body.append(field(rule && occurrenceDate ? "THIS OCCURRENCE" : "", acts, true));
}

/* ── small builders ────────────────────────────────────────────────── */
/** 1 → "1st", 22 → "22nd", 13 → "13th". */
function ordinal(n) {
  const teen = n % 100 >= 11 && n % 100 <= 13;
  return `${n}${teen ? "th" : ({ 1: "st", 2: "nd", 3: "rd" })[n % 10] || "th"}`;
}

function field(label, control, stacked) {
  const row = el("div", stacked ? "fieldStacked" : "field");
  if (label) row.append(el("span", "fieldLabel", label));
  row.append(control);
  return row;
}

function stepper(value, fmt, onSet, step) {
  const wrap = el("div", "stepper");
  const minus = el("button", "sbtn", "−");
  const val = el("span", "sval", fmt(value));
  const plus = el("button", "sbtn", "+");
  minus.onclick = () => onSet(Math.max(0, (value || 0) - step) || null);
  plus.onclick = () => onSet((value || 0) + step);
  wrap.append(minus, val, plus);
  return wrap;
}

function timeEditor(rule, onSet) {
  const wrap = el("div", "timeEdit");
  // Minutes from midnight, reading an end of 00:00 as the midnight after the
  // start (1440), as the backend does. Steps used to wrap round the clock, so
  // stretching an 11pm-12am block saved an end of 12:30am -- before its start --
  // and the rule vanished.
  const toMin = (hhmm) => { const [h, m] = hhmm.split(":").map(Number); return h * 60 + m; };
  const pad = (n) => String(n).padStart(2, "0");
  const fmt = (t) => (t === 1440 ? "00:00" : `${pad(Math.floor(t / 60))}:${pad(t % 60)}`);
  const start = toMin(rule.start_time);
  const end = toMin(rule.end_time) === 0 && start > 0 ? 1440 : toMin(rule.end_time);
  // A block stays inside its day; a step that would cross midnight does nothing.
  const set = (s, e) => {
    if (s < 0 || e > 1440 || e <= s || (s === start && e === end)) return;
    onSet(fmt(s), fmt(e));
  };
  const label = el("span", "sval", rangeHHMM(rule.start_time, rule.end_time));
  const earlier = el("button", "sbtn", "−");
  const later = el("button", "sbtn", "+");
  const shorter = el("button", "sbtn", "−LEN");
  const longer = el("button", "sbtn", "+LEN");
  earlier.onclick = () => set(start - 30, end - 30);
  later.onclick = () => set(start + 30, end + 30);
  shorter.onclick = () => set(start, end - 30);
  longer.onclick = () => set(start, Math.min(end + 30, 1440));
  wrap.append(earlier, label, later, shorter, longer);
  return wrap;
}

function alertLine(body, msg) {
  const existing = body.querySelector(".detailError");
  if (existing) existing.remove();
  const e = el("div", "detailError", msg);
  body.prepend(e);
}
