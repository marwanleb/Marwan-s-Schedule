# What the capture parser should and should not do

This is the contract for the one thing the app asks you to learn: typing a line
and having it understood. It is written for someone testing the app from the
outside, so it describes behaviour, never implementation.

Every case below is checked against the real parser by
`python tools/behaviour_check.py`. If the document and the code disagree, that
command fails. Do not fix a mismatch by editing this file until you are sure
the document is the thing that is wrong.

All examples assume **today is Monday 31 August 2026** and the machine is in
Austin (`America/Chicago`).

---

## The three promises

Everything else follows from these. If you find a case that breaks one, that is
a bug worth reporting even if it is not listed below.

**1. Nothing is ever lost.** Whatever the parser does not understand stays in
the title, exactly as typed. There is no input that produces an error, and no
input that silently discards a word.

**2. It never guesses.** A detail is only extracted when it is unambiguous. A
typo, an unusual spelling, or an unfamiliar phrase leaves the text alone rather
than producing a plausible wrong answer. A wrong date is worse than no date.

**3. Ordinary sentences survive.** Details are read from the **end** of the
line, and scanning stops at the first word that is not a recognised detail.
This is why prose containing date-shaped words is untouched.

That third promise is the reason for the biggest limitation, below.

---

## Details are read from the end

The parser reads backwards from the end of the line and stops at the first word
it does not recognise. Everything to the left is the title.

| input | expected | why |
|---|---|---|
| `math hw fri 5pm ~2h #math` | title=math hw | details all trail the title |
| `9/2 8pm study session` | title=<same> | details lead, so nothing is read |
| `meet Sarah about the March report` | title=<same> | scanning stops at "report" |

**This is a deliberate trade, not an oversight.** Reading details from anywhere
in the line would mangle the third example — "March" is a month name. Putting
the date at the front is the one phrasing that does not work, and it is the
price of ordinary sentences being safe.

---

## What it should understand

### Dates

Day comes before month. Where one number cannot be a month, that settles it
regardless of order.

| input | expected | note |
|---|---|---|
| `x today` | due=2026-08-31 | |
| `x tomorrow` | due=2026-09-01 | |
| `x fri` | due=2026-09-04 | the next one |
| `x friday` | due=2026-09-04 | written out |
| `x tues` | byday=1 | common abbreviation |
| `x thurs` | byday=1 | |
| `x 2026-09-15` | due=2026-09-15 | |
| `x 2/9` | due=2026-09-02 | day first |
| `x 2/9/2026` | due=2026-09-02 | |
| `x 2-9-2026` | due=2026-09-02 | |
| `x 2.9.2026` | due=2026-09-02 | dots, as most of Europe writes |
| `x 02/09/26` | due=2026-09-02 | two-digit year |
| `x 25/12` | due=2026-12-25 | 25 can only be a day |
| `x 12/25/2026` | due=2026-12-25 | so can 25 here |
| `x 2 sep` | due=2026-09-02 | month name |
| `x 2 September 2026` | due=2026-09-02 | |
| `x Sep 2` | due=2026-09-02 | either order |
| `x September 2, 2026` | due=2026-09-02 | with a comma |
| `x 2nd September` | due=2026-09-02 | ordinal |
| `x 1 oct 9am` | due=2026-10-01, at=09:00 | a time after a written date |
| `x oct 1 9am` | due=2026-10-01, at=09:00 | either order |
| `rent monthly 1 oct 9am` | title=rent, repeats=true, at=09:00 | the help's own example |

### Times

| input | expected | note |
|---|---|---|
| `x 17:00` | at=17:00 | |
| `x 5pm` | at=17:00 | |
| `x 5 pm` | at=17:00 | as its own word |
| `x 5p` | at=17:00 | |
| `x 8 p.m.` | at=20:00 | with stops |
| `x 20:00:00` | at=20:00 | seconds ignored |
| `x noon` | at=12:00 | |
| `x midday` | at=12:00 | |
| `x midnight` | at=00:00 | |
| `x 9:00-10:15` | span=09:00-10:15 | a range |
| `x 9:30a-11a` | span=09:30-11:00 | |

### Repeating

Recurrence is **stated**, never inferred from the shape of the line.

| input | expected | note |
|---|---|---|
| `x every tue` | repeats=true | the keyword |
| `x weekly tue` | repeats=true | synonym |
| `x daily` | repeats=true, byday=7 | every day |
| `rent monthly 1/10/2026` | repeats=true, byday=0, due=none | the date's day, every month |
| `x monthly` | repeats=true, byday=0 | today's day of the month |
| `x mondays` | repeats=true | the plural says it |
| `gym every mon wed fri ~1h` | title=gym, repeats=true, byday=3 | |
| `gym mon/wed/fri 6am` | title=gym, byday=3 | slashes |
| `x fri` | repeats=false | one weekday is a date, not a repeat |
| `x mon wed` | repeats=true | two can only mean a repeat |

### Task or block

`due` and `on` say which kind of thing this is. Without either, a time **range**
makes it a block and anything else is a task.

`on` says *when something happens*, not that it stops being something to
finish, so it does both: an hour on the week and a line you can tick off. They
are one object, so ticking either finishes both. It is due when its block
starts, whether that was given as a time or a range, and moving the block moves
the due time with it.

| input | expected | note |
|---|---|---|
| `essay due friday` | listed=true, scheduled=false | a deadline, in the list |
| `standup on friday 9am` | listed=true, scheduled=true | on the week and tickable |
| `lab report due wed by 11:59 pm` | listed=true | "due" wins over "by" |
| `haircut on friday` | listed=true, scheduled=false | no time, so nothing to place |
| `MATH210 every mon 9:00-10:15` | listed=false | a bare range is a block only |
| `renew parking` | listed=true | |

### Everything else

| input | expected | note |
|---|---|---|
| `x ~2h` | est=120 | estimate |
| `x ~90m` | est=90 | |
| `x ~1.5h` | est=90 | |
| `problem set fri ~2h #math` | title=problem set, est=120 | tag stays a tag |
| `lecture every mon 9:00-10:00 @Hall 2.106` | loc=Hall 2.106 | places contain spaces |
| `coffee @the corner cafe` | loc=the corner cafe | |
| `meet bob @ starbucks` | loc=starbucks | a bare @ counts |
| `meet bob @starbucks 3pm` | loc=starbucks, at=15:00 | place stops at the time |
| `x @Hall 2.106 #work` | loc=Hall 2.106 | and at a tag |
| `!! renew parking` | title=renew parking | priority marker |
| `wake up daily 08:00 #floating` | title=wake up | follows the machine when you travel |

Filler words between a task and its details — `at`, `on`, `by`, `due`, `from` —
do not stop the scan.

| input | expected | note |
|---|---|---|
| `flight to paris on sept 3 at 6:45am` | title=flight to paris, at=06:45 | |
| `lunch at noon monday` | title=lunch, at=12:00 | |

---

## What it should NOT do

These matter more than the section above. A parser that grabs too much is worse
than one that grabs too little, because the mistake is invisible.

### Prose must survive untouched

| input | expected | why it is tempting |
|---|---|---|
| `meet Sarah about the March report` | title=<same> | "March" is a month |
| `buy milk for tomorrow's breakfast` | title=<same> | "tomorrow's" |
| `read the daily news` | title=<same> | "daily" |
| `read the monthly report` | title=<same> | "monthly" |
| `water the plants every day` | title=<same> | "every" |
| `finish the essay in March` | due=none | a month with no day is not a date |
| `x September` | title=<same> | nor on its own |
| `think about it` | title=<same> | ends in a preposition |
| `meet him by` | title=<same> | ends in a filler word |
| `email the notes I got from` | title=<same> | |
| `essay due` | title=<same> | "due" with nothing to be due |
| `the thing to do` | title=<same> | |

### Numbers that are not dates or times

| input | expected | why it is tempting |
|---|---|---|
| `upgrade to 1-2-3` | title=<same> | looks like a date |
| `bump rustc to 1.95.0` | title=<same> | a version |
| `read chapters 2-9` | title=<same> | a dash range, not 2 September |
| `call him on 5` | title=<same> | a bare number is not a time |
| `x 45-99-2026` | title=<same> | no such date |

### Addresses and markers

| input | expected | why it is tempting |
|---|---|---|
| `email bob@example.com about the lease` | loc=none | `@` mid-word is not a place |

### Never lose a capture

| input | expected | why |
|---|---|---|
| `!!!` | title=!!! | stripping the marker would leave nothing |
| `renew parking` | consumed=0 | plain text is left completely alone |

---

## Known gaps

Deliberate — do not report these as bugs:

- **Details must trail the title.** `9/2 study session` extracts nothing. See
  the trade above.
- **`2-9` is not a date.** A dash between two small numbers is a range far more
  often than a date. Use `2/9` or `2.9`.
- **A bare month is not a date.** `September` alone has no day in it.
- **`2000` is not a time.** Too easily a year or a plain number.
- **`@8pm` is a place, not a time.** `@` marks a location.

Not yet supported — worth reporting if they get in your way, but already known:

- Relative phrases: `next week`, `in two days`, `end of the month`
- Spoken times: `half past eight`, `eight o'clock`
- `20h` (French style), `8.00pm`
- Repeats other than weekly: `every other friday`, `monthly`
- Durations as ranges: `jan 5-12` for a trip spanning days

---

## How to test this

The app echoes what it understood under the input box after you add something.
That is the fastest way to see a mis-parse.

From a terminal, against a scratch database so you never touch real data:

```
sched.exe --db %TEMP%\schedule-test.db add "<your line>"
sched.exe --db %TEMP%\schedule-test.db export --json
```

**A failure is when the title still contains something that should have been
understood** — or, worse, when a detail was extracted and is wrong.

The most valuable cases are lines you would actually type without thinking
about it. Sloppiness is welcome: inconsistent capitals, missing punctuation,
abbreviations, extra words. Lines that are pure prose and should be left alone
are as useful as lines that should parse.
