use crate::parse::{parse, Parsed};
use chrono::NaiveDate;

/// How much of the list to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListScope {
    /// Due today or already overdue.
    Today,
    /// Due within the next seven days.
    Week,
    /// Everything still open, dated or not.
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    List(ListScope),
    Week,
    /// Mark done by title prefix.
    Done(String),
    /// Delete by title prefix.
    Delete(String),
    /// Bring back whatever was deleted last.
    Undo,
    Capture(Parsed),
}

/// Single entry point for typed input, wherever it arrives from: the Telegram
/// bot, the CLI, or the overlay. Keeping dispatch here means those three cannot
/// drift apart.
pub fn interpret(input: &str, today: NaiveDate) -> Command {
    let text = input.trim();
    // Slash-prefixed forms are conventional in chat clients.
    let bare = text.strip_prefix('/').unwrap_or(text);
    let lower = bare.to_ascii_lowercase();

    match lower.as_str() {
        // Telegram sends /start on its own the first time a bot is opened.
        "help" | "?" | "h" | "start" | "hi" | "hello" => return Command::Help,
        "list" | "list all" => return Command::List(ListScope::All),
        "list today" | "today" => return Command::List(ListScope::Today),
        "list week" => return Command::List(ListScope::Week),
        // "week" on its own is the schedule, which is a different question
        // from "what is due this week".
        "week" => return Command::Week,
        "undo" => return Command::Undo,
        _ => {}
    }

    // "done: x" and "done x" both mark x complete. Matched on a word boundary
    // so "done" inside a task title does not trigger it.
    for prefix in ["done:", "done"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                return Command::Done(bare[prefix.len()..].trim().to_string());
            }
        }
    }

    // Deleting wants the colon, or the slash form a chat client offers.
    // Without one, "delete old emails" is a task like any other and must
    // still be captured.
    if let Some(rest) = lower.strip_prefix("delete:") {
        return Command::Delete(bare[bare.len() - rest.len()..].trim().to_string());
    }
    if text.starts_with('/') {
        if let Some(rest) = lower.strip_prefix("delete") {
            if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace()) {
                return Command::Delete(bare[bare.len() - rest.len()..].trim().to_string());
            }
        }
    }

    Command::Capture(parse(text, today))
}

/// The one description of the syntax. Printed by `help`, by the CLI, and
/// shipped in AGENT.md, so there is a single thing to keep true.
pub fn help_text() -> String {
    "Type what you want to remember. Plain text always works:

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

  help            this text
  list            everything still open
  list today      due today or overdue
  list week       due in the next seven days
  week            this week's schedule
  done: <text>    tick off the first task matching <text>
  delete: <text>  delete the task matching <text>
  undo            bring back the last thing deleted

Leave things out and you are asked for them one at a time — when, what
time, how long — and `-` skips any of them. A complete line is never
questioned, so the asking is the price of being terse rather than a tax
on every capture. Send `cancel` to drop a half-finished one.

Ten minutes before anything on the week starts, or anything in the list
falls due, this chat gets a reminder. Nothing to switch on — it pushes to
whoever last spoke to it.
"
    .to_string()
}
