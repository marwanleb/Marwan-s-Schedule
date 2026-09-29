# AGENT.md — driving `sched`

This is the whole interface for an agent populating Marwan's schedule. The app
never fetches anything by itself; nothing appears unless you put it there.

## The one rule that matters

**Every imported record needs a stable `external_id` that you derive from the
source, never generate randomly.** It is what makes a second run a no-op instead
of a second copy of everything.

    canvas:assignment:88213     good — stable across re-scans
    canvas:STAT240-pset-4          good — stable if the source names it that way
    a fresh uuid each run       WRONG — this is how the previous app ended up
                                posting every deadline twice

Re-running an identical import changes nothing. A record whose due date changed
updates in place. Items Marwan typed himself have no `external_id` and are never
touched by an import.

## Check before you add

    sched list --json
    sched export --json

Prefer `sched import` over repeated `sched add` for anything derived from an
external source: `add` has no key, so calling it twice really does create two
items. `add` is for one-off human-style capture.

## Import file

A JSON array. `external_id` and `title` are required; everything else optional.

```json
[
  {
    "external_id": "gcal:7f3a9c2e",
    "title": "STAT240 problem set 4",
    "due_at": "2026-09-04T23:59:00-05:00",
    "estimate_min": 120,
    "tags": ["math"]
  },
  {
    "external_id": "gcal:phys201",
    "title": "PHYS201",
    "tags": ["class"],
    "location": "Hall 2.106",
    "repeat": {
      "byday": ["mon", "wed"],
      "start_time": "10:30",
      "end_time": "12:00",
      "tz": "America/Chicago"
    }
  }
]
```

Then:

    sched import schedule.json

**`repeat` is how a timetable goes in.** A weekly class is one record with a
repeat rule, not fifteen dated copies — one row that renders every week and can
be edited in one place. An item with a `repeat` is treated as a commitment: it
occupies the week and stays out of the to-do list.

- `byday` — any of `mon tue wed thu fri sat sun`, at least one.
- `start_time` / `end_time` — 24-hour `HH:MM`, wall clock.
- `tz` — an IANA name (`America/Chicago`, `Europe/Paris`, `Asia/Beirut`). It
  fixes the class to that city's clock, so it still reads correctly from
  another country. Omit it and the time follows whatever machine is showing it,
  which is what you want for something like a wake-up alarm. **Never an
  abbreviation** — `CT` is rejected.
- Dropping `repeat` on a later import removes the rule, so a cancelled class
  stops appearing.

`due_at` is for one-off deadlines and wants a full RFC3339 timestamp including
the offset. `starts_at` and `ends_at` (both RFC3339, both or neither, not with
`repeat`) put a single dated block on the week, like a meeting. Such a record
stays out of the to-do list, and a re-import moves the block back to where the
record says. `tags` are free-form; the four that also set a display colour are
`work`, `life`, `body`, `social`.

The batch applies completely or not at all. A record missing `external_id`, or
carrying an unknown time zone, is refused and **nothing** is written — a
half-applied import is worse than a failed one, because it looks like it worked.

## Where to put the file, and how to run it

`sched.exe` lives beside the app:

    %USERPROFILE%\marwans-schedule\sched.exe

From a source checkout it is `target\release\sched.exe` instead. Either way,
import files go in a dated run directory next to the binary:

    sched-sync\runs\<YYYY-MM-DD>\import.json

One directory per sync, named for the date it was produced. A run is then
something you can look at afterwards, re-apply verbatim, or diff against the
next one — and because every record carries an `external_id`, re-applying an
old run is harmless.

### PowerShell

```powershell
Set-Location "$env:USERPROFILE\marwans-schedule"
.\sched.exe import sched-sync\runs\2026-09-09\import.json
```

```
imported  2 added, 0 updated
```

Run it again on the same file and it says `0 added, 2 updated`. That is the
whole point: re-running is safe.

Three things trip agents up on Windows:

- **`.\` is required.** PowerShell will not run an executable from the current
  directory without it. Bare `sched.exe import ...` fails unless the folder is
  on `PATH`.
- **`cd /d` is cmd.exe, not PowerShell.** Use `Set-Location`, or plain `cd`.
- **`&&` does not chain commands in Windows PowerShell 5.1.** Use `;`, or
  `if ($?) { ... }` when the second should only run if the first succeeded.

A byte-order mark is **not** one of them, though it was until recently. Both
`Set-Content -Encoding utf8` and `Out-File -Encoding utf8` write one in
PowerShell 5.1, and the import used to be refused with `expected value at line
1 column 1` — an error pointing at a character you cannot see. `sched import`
now strips it, so write the file whichever way is convenient.

### Rehearse against a scratch store first

`--db` points at any file, so a run can be applied somewhere harmless and
inspected before it goes near the real store:

```powershell
$scratch = "$env:TEMP\dry-run.db"
Remove-Item $scratch -ErrorAction SilentlyContinue
.\sched.exe --db $scratch import sched-sync\runs\2026-09-09\import.json
.\sched.exe --db $scratch week
```

If that reads correctly, run the same command again without `--db`.

### Other shells

In bash or zsh it is `./sched import sched-sync/runs/2026-09-09/import.json`,
and none of the four traps apply.

## Commands

    sched add "<text>"          capture one line, same syntax as the app
    sched list [--today]        open items
    sched week [--on DATE]      the week's schedule
    sched done "<title start>"  tick off; refuses if the prefix is ambiguous
    sched except <id> <date>    cancel one occurrence of a repeat
    sched rm <id>               delete
    sched import <file.json>    bulk upsert (see above)
    sched sync-ics [<url>]      mirror a published calendar; see below
    sched export                every item as JSON
    sched stats [--tag T]       how much longer things take than estimated
    sched help                  syntax reference

Add `--json` to any read command for machine-readable output. `--db <path>`
targets a specific store; the default is the app's own.

Exit codes: `0` success, `1` refused (bad input, nothing written), `2` not found
or ambiguous.

## Calendar feeds

A published calendar (Outlook's "Publish a calendar" ICS link, Google's secret
iCal address) does not go through an import file. `sched sync-ics <url>` reads
it, saves the address, and from then on the app re-reads it every 15 minutes.

Every occurrence from a week back to 120 days ahead becomes its own block,
tagged `calendar`, keyed `ics:<UID>` or `ics:<UID>@<original start>`. Unlike
`import`, a sync also **deletes**: an `ics:` event that is upcoming and no
longer in the feed is removed. Past ones are kept. All-day events are skipped.
One feed at a time; a new address replaces the old.

    sched sync-ics                reuse the saved address
    sched sync-ics --file x.ics   a downloaded file, not saved
    sched sync-ics --forget       stop, and remove its upcoming events

Do not hand-write `ics:` records into an import file; the next sync owns them.

## Capture syntax

`sched add` uses exactly the same parser as the app and the Telegram bot, so
anything below also works when typed by hand.

```
Type what you want to remember. Plain text always works:

> renew parking

Add details on the end. Anything the parser does not recognise stays
in the title, so ordinary sentences are safe.

  when        fri · friday · tues · mondays · tomorrow · today
              2 sep · 2 September 2026 · Sep 2
              2026-09-15 · 2-9-2026 · 2/9 · 2.9.26   (day first)
  time        5pm · 5 pm · 5p · 17:00 · noon · midnight
  a block     9:00-10:15 · 10:30-12:00p   (scheduled; kept out of the list)
  kind        due fri      → a task with a deadline
              on fri 9am   → an hour on the week, still tickable
  repeating   every        (say it plainly; one weekday is enough)
              mondays      (the plural says it too)
              daily        (every day of the week)
              monthly      (on its date's day each month; the 31st
                            means the last day of a shorter month)
  estimate    ~2h · ~90m · ~1.5h
  tag         #math        (#work #life #body #social also set the colour)
  place       @Hall 2.106   (runs to the end of the line, spaces and all)
  travel      #floating    (follows this machine; otherwise it stays fixed
                            to the zone you created it in)
  important   !! in front

> math hw fri 5pm ~2h #math
> trash every tue 20:00
> PHYS201 every monday wednesday 10:30-12:00p @Hall 2.106
> gym every mon wed fri ~1h #body
> wake up daily 08:00 #floating
> rent monthly 1 oct 9am
> pick up a friend from the airport 2 Sep 2026 8:00 pm
> dentist 14 october 9:30 am
> lab report due wed by 11:59 pm
> standup on friday 9am
> gym mon/wed/fri 6am
> !! renew parking tomorrow

Commands:

  help          this text
  list          everything still open
  list today    due today or overdue
  list week     due in the next seven days
  week          this week's schedule
  done: <text>  tick off the first task matching <text>

Leave things out and you are asked for them one at a time — when, what
time, how long — and `-` skips any of them. A complete line is never
questioned, so the asking is the price of being terse rather than a tax
on every capture. Send `cancel` to drop a half-finished one.

Ten minutes before anything on the week starts, or anything in the list
falls due, this chat gets a reminder. Nothing to switch on — it pushes to
whoever last spoke to it.
```

## What not to do

- Do not invent deadlines. Only add what you actually found.
- Do not use `add` in a loop over scraped data; use `import`.
- Do not retry a failed `import` by switching to `add` — fix the record and
  re-run the import, which is safe.
