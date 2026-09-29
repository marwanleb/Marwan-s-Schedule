// Stands in for the Tauri backend so the real frontend can run in a browser.
//
// Two modes. By default the week is populated with an invented term — enough
// to see layout, colour and density, and what the README screenshots are made
// from. `?empty` gives a blank week instead, which is what you want when
// testing a gesture that needs a free hour to click into.
//
// Every invoke is logged and recorded in `window.__calls`, so a gesture that
// should reach the store but does not is visible as an absence.

const EMPTY = new URLSearchParams(location.search).has("empty");

// Everything is placed against the real week, so the fixture looks the same
// whenever it is run: classes on their weekdays, everything else around today.
const NOW = new Date();
const MON = new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate() - ((NOW.getDay() + 6) % 7));
/** Today's offset from that Monday. */
const T = (NOW.getDay() + 6) % 7;
const MONDAY = `${MON.getFullYear()}-${String(MON.getMonth() + 1).padStart(2, "0")}-${String(MON.getDate()).padStart(2, "0")}`;
const iso = (offset) => {
  const d = new Date(MON.getFullYear(), MON.getMonth(), MON.getDate() + offset);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};
// Local wall-clock times with no offset, so they read the same in any season.
const at = (offset, hhmm) => `${iso(offset)}T${hhmm}:00`;

let seq = 0;
const uid = (p) => `${p}_${(++seq).toString().padStart(4, "0")}`;

/** Every block belongs to an item; the popover looks it up by id. */
const blockItems = [];
function itemFor(id, title, category, place, recurs, estimate_min) {
  blockItems.push({
    id, title, tags: [], category, location: place,
    listed: false, due_at: null, estimate_min, recurs,
    done: false, done_on: [], completed_at: null,
    source: "self", external_id: null, spent_sec: 0,
  });
  return id;
}

/** A class: the same item placed on several days. */
function course(title, days, start, end, place, category) {
  const id = itemFor(uid("itm"), title, category, place, true, null);
  return days.flatMap((d) => [d, d + 7]).map((d) => ({
    id: `plc_${iso(d)}_${id}`,
    item_id: id,
    title,
    category,
    location: place,
    starts_at: at(d, start),
    ends_at: at(d, end),
    origin: "recurrence",
    pinned_tz: "America/Chicago",
    foreign: false,
    recurs: true,
    done: false,
  }));
}

function oneOff(title, day, start, end, place, category, done = false) {
  const id = itemFor(uid("itm"), title, category, place, false, null);
  return [{
    id: uid("plc"),
    item_id: id,
    title,
    category,
    location: place,
    starts_at: at(day, start),
    ends_at: at(day, end),
    origin: "oneoff",
    pinned_tz: null,
    foreign: false,
    recurs: false,
    done,
  }];
}

const blocks = EMPTY ? [] : [
  ...course("PHYS201", [0, 2], "10:30", "12:00", "Hall 2.106", "work"),
  ...course("MATH210", [0, 2, 4], "09:00", "10:00", "Hall 1.204", "work"),
  ...course("STAT240 lab", [3], "14:00", "17:00", "Lab 3.210", "work"),
  ...course("gym", [1, 3, 4], "07:00", "08:00", null, "body"),
  ...oneOff("coffee with Sam", T + 1, "15:00", "16:00", "the corner cafe", "social"),
  ...oneOff("dentist", T + 2, "16:30", "17:15", null, "life"),
  ...oneOff("dinner at Ana's", T + 3, "19:00", "21:00", null, "social"),
  ...oneOff("seminar reading", T, "20:00", "21:30", null, "work"),
];

/** Seven days from whatever start the app asks for, as the real backend does. */
function weekFrom(anchor) {
  const [y, m, d] = (anchor || MONDAY).split("-").map(Number);
  const days = [];
  for (let i = 0; i < 7; i++) {
    const day = new Date(y, m - 1, d + i);
    const key = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;
    days.push({ date: key, placements: blocks.filter((p) => p.starts_at.startsWith(key)) });
  }
  return { anchor: days[0].date, viewing_tz: "America/Chicago", days, diagnostics: [] };
}

function todo(title, dueOffset, dueTime, opts = {}) {
  return {
    id: uid("itm"),
    title,
    tags: opts.tags || [],
    category: opts.category || null,
    location: null,
    listed: true,
    due_at: dueOffset === null ? null : at(dueOffset, dueTime),
    estimate_min: opts.estimate_min || null,
    recurs: false,
    done: opts.done || false,
    done_on: [],
    // Recent, not fixed: a ticked item clears itself after an hour, so a
    // hardcoded timestamp would have aged out by the time anyone looked.
    completed_at: opts.done ? new Date(Date.now() - 10 * 60000).toISOString() : null,
    source: "self",
    external_id: null,
    spent_sec: opts.spent_sec || 0,
  };
}

// Due dates sit around today, so every bucket and both chip styles appear.
const todos = EMPTY ? [] : [
  todo("PHYS201 problem set 4", T, "23:59", { estimate_min: 120, category: "work", spent_sec: 2700 }),
  todo("pay the phone bill", T, "18:00", { category: "life" }),
  todo("email the lab about lost keys", T, "17:00", { estimate_min: 15, category: "work", done: true }),
  todo("MATH210 quiz 3", T + 1, "23:59", { estimate_min: 60, category: "work" }),
  todo("draft the internship email", T + 2, "12:00", { estimate_min: 30, category: "work" }),
  todo("return the library books", T + 3, "17:00", { category: "life" }),
  todo("farmers market", T + 4, "10:00", { category: "life" }),
  todo("call home", T + 4, "18:00", { estimate_min: 30, category: "social" }),
  todo("book flights home", T + 5, "23:59", { estimate_min: 45, category: "life" }),
  todo("renew passport", T + 27, "12:00", { category: "life" }),
  todo("renew parking", null, null, { category: "life" }),
  todo("fix the bike", null, null, { estimate_min: 90, category: "body" }),
  todo("start the reading list", null, null, {}),
];
// Blocks first so their ids resolve; they are listed:false and stay out of the
// to-do pane.
const items = EMPTY ? [] : [...blockItems, ...todos];

const session = EMPTY ? null : {
  id: "ses_0001",
  item_id: todos[0].id,
  title: todos[0].title,
  started_at: new Date(Date.now() - 45 * 60000).toISOString(),
  estimate_min: 120,
  spent_sec: 2700,
};

const deleted = new Set();
window.__calls = [];
window.__TAURI__ = {
  core: {
    invoke: async (cmd, args) => {
      window.__calls.push({ cmd, args });
      console.log("[INVOKE]", cmd, JSON.stringify(args || {}));
      // A check can stand in for any command without editing this file:
      //   window.__override = { cmd_get_recurrence: () => ({ ...a rule... }) }
      if (window.__override && window.__override[cmd]) return window.__override[cmd](args);
      switch (cmd) {
        case "cmd_get_week": return weekFrom(args && args.anchor);
        case "cmd_get_items": return items.filter((i) => !deleted.has(i.id));
        // Soft, like the store: the row is kept so undo can bring it back.
        case "cmd_delete": deleted.add(args.id); return null;
        case "cmd_restore": deleted.delete(args.id); return null;
        case "cmd_active_session": return session;
        case "cmd_stats": return EMPTY ? { phrase: null, n: 0 } : { phrase: "about a third longer", n: 18 };
        case "cmd_data_version": return 1;
        // The popup's time-zone picker iterates this; null crashed it whenever
        // an item had a rule. The real backend always returns a list.
        case "cmd_zones": return ["America/Chicago", "Europe/Paris", "Asia/Beirut"];
        // A rough stand-in for core::compose::line, enough to see the preview
        // assemble. The real one is Rust and round-trip tested against parse.
        case "cmd_compose": {
          const f = args.fields;
          const bits = [];
          if (f.priority) bits.push("!!");
          bits.push(f.title);
          if (f.monthly) bits.push("monthly");
          else if (f.repeat && f.repeat.length) bits.push("every " + f.repeat.join(" "));
          else if (f.date) bits.push(f.kind === "block" ? "on" : "due");
          if (f.date && (f.monthly || !(f.repeat && f.repeat.length))) {
            const [y, m, d] = f.date.split("-");
            bits.push(`${d}/${m}/${y}`);
          }
          // 12-hour, like core::compose::line.
          const t12 = (hhmm) => {
            const [h, m] = hhmm.split(":").map(Number);
            const mer = h < 12 ? "am" : "pm";
            return m ? `${h % 12 || 12}:${String(m).padStart(2, "0")}${mer}` : `${h % 12 || 12}${mer}`;
          };
          if (f.at) bits.push(f.span_end ? `${t12(f.at)}-${t12(f.span_end)}` : t12(f.at));
          if (f.estimate_min) {
            bits.push(f.estimate_min % 60 === 0 && f.estimate_min >= 60
              ? `~${f.estimate_min / 60}h` : `~${f.estimate_min}m`);
          }
          if (f.tag) bits.push("#" + f.tag);
          if (f.place) bits.push("@" + f.place);
          return bits.join(" ");
        }
        case "cmd_move_placement":
        case "cmd_move_occurrence":
          return "plc_stub";
        case "cmd_help": return "run `sched help` for the real thing";
        case "cmd_add": return {
          item: { ...todo(args.text, null, null, {}), id: "itm_new" },
          consumed: ["stub"], byday: [], span: null, location: null,
        };
        default: return null;
      }
    },
  },
  event: { listen: () => {} },
};
window.addEventListener("error", (e) => console.log("[WINDOW ERROR]", e.message, e.filename + ":" + e.lineno));
window.addEventListener("unhandledrejection", (e) =>
  console.log("[UNHANDLED REJECTION]", e.reason && e.reason.stack ? e.reason.stack : String(e.reason)));
