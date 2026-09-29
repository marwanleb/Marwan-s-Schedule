use chrono::NaiveDate;
use ms_core::{help_text, interpret, Command};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()
}

#[test]
fn help_is_reachable_by_typing_it() {
    for input in ["help", "HELP", "  help  ", "?", "/help"] {
        assert_eq!(interpret(input, today()), Command::Help, "{input:?} should ask for help");
    }
}

#[test]
fn the_other_commands_are_recognised() {
    assert_eq!(interpret("list", today()), Command::List(ms_core::ListScope::All));
    assert_eq!(interpret("week", today()), Command::Week);
    assert_eq!(interpret("done: math hw", today()), Command::Done("math hw".into()));
    assert_eq!(interpret("done math hw", today()), Command::Done("math hw".into()));
    assert_eq!(interpret("undo", today()), Command::Undo);
}

#[test]
fn delete_needs_its_colon_or_its_slash() {
    assert_eq!(interpret("delete: Math HW", today()), Command::Delete("Math HW".into()));
    assert_eq!(interpret("/delete math hw", today()), Command::Delete("math hw".into()));
    assert_eq!(interpret("/delete", today()), Command::Delete(String::new()));
    // Without either, it is a task about deleting something.
    let Command::Capture(p) = interpret("delete old emails fri", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "delete old emails");
}

#[test]
fn anything_else_is_a_capture() {
    let Command::Capture(p) = interpret("math hw fri ~2h", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "math hw");
}

/// A word that merely starts with a command name is still a task.
#[test]
fn command_names_inside_a_task_do_not_trigger() {
    let Command::Capture(p) = interpret("helpline callback fri", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "helpline callback");

    let Command::Capture(p) = interpret("list the christmas presents", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "list the christmas presents");
}

/// Help that drifts from behaviour is worse than none: every example printed
/// in the help text must actually parse the way it claims.
#[test]
fn every_example_in_the_help_text_really_works() {
    let text = help_text();
    let examples: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("> "))
        .collect();

    assert!(examples.len() >= 5, "help should carry worked examples, found {}", examples.len());

    let markers = ['~', '#', '@', '!', ':', '/', '-'];
    let mut demonstrated = 0;

    for ex in examples {
        let Command::Capture(p) = interpret(ex, today()) else {
            continue; // command examples like "list" are fine
        };
        assert!(!p.title.is_empty(), "help example {ex:?} leaves no title");

        // An example that shows off syntax must actually exercise it; one that
        // shows plain text deliberately must not.
        let shows_syntax = ex.chars().any(|c| markers.contains(&c) || c.is_ascii_digit())
            || ex.split_whitespace().any(|w| {
                ["every", "today", "tomorrow", "mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                    .contains(&w.to_ascii_lowercase().as_str())
            });
        if shows_syntax {
            demonstrated += 1;
            assert!(
                !p.consumed.is_empty(),
                "help example {ex:?} shows syntax the parser does not recognise"
            );
        } else {
            assert_eq!(p.title, ex, "plain-text example {ex:?} should be kept whole");
        }
    }

    assert!(demonstrated >= 4, "only {demonstrated} examples actually exercise the syntax");
}

/// Telegram sends /start by itself when you first open a bot. Treating it as
/// something to remember is not a sensible first impression.
#[test]
fn start_is_a_greeting_not_a_task() {
    for input in ["/start", "start", "/help", "hi", "hello"] {
        assert_eq!(interpret(input, today()), Command::Help, "{input:?}");
    }
}

/// But it is only a greeting on its own — a real task beginning with the word
/// is still a task.
#[test]
fn a_task_beginning_with_start_is_still_a_task() {
    let Command::Capture(p) = interpret("start the laundry tomorrow", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "start the laundry");
}

/// `list` takes a scope, so the reply can be as short as the question.
#[test]
fn list_can_be_narrowed() {
    use ms_core::ListScope;
    assert_eq!(interpret("list", today()), Command::List(ListScope::All));
    assert_eq!(interpret("list all", today()), Command::List(ListScope::All));
    assert_eq!(interpret("list today", today()), Command::List(ListScope::Today));
    assert_eq!(interpret("list week", today()), Command::List(ListScope::Week));
    assert_eq!(interpret("/list today", today()), Command::List(ListScope::Today));
    assert_eq!(interpret("LIST WEEK", today()), Command::List(ListScope::Week));
}

/// "today" on its own is the short way to ask the same thing.
#[test]
fn today_alone_is_the_short_form() {
    use ms_core::ListScope;
    assert_eq!(interpret("today", today()), Command::List(ListScope::Today));
}

/// But "week" alone still means the schedule — a different question.
#[test]
fn week_alone_is_still_the_schedule() {
    assert_eq!(interpret("week", today()), Command::Week);
}

/// A task that merely begins with the word is still a task.
#[test]
fn list_inside_a_task_does_not_trigger() {
    let Command::Capture(p) = interpret("list the christmas presents", today()) else {
        panic!("should be a capture");
    };
    assert_eq!(p.title, "list the christmas presents");
}

/// Every example in the help must be understood completely: no word left in
/// its title reads as a detail on its own. Checking only that something was
/// recognised let "rent monthly 1 oct 9am" through while it filed a task
/// called "rent monthly 1" with no repeat -- and re-reading the whole title
/// misses it too, because the stranded "monthly" is not the last word. So
/// every prefix of the title is re-read, which puts each word at the end once.
#[test]
fn every_help_example_is_understood_completely() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
    for line in ms_core::help_text().lines() {
        let Some(example) = line.strip_prefix("> ") else { continue };
        let title = ms_core::parse(example, today).title;
        let words: Vec<&str> = title.split_whitespace().collect();
        for k in 1..=words.len() {
            let prefix = words[..k].join(" ");
            let again = ms_core::parse(&prefix, today);
            assert!(
                again.consumed.is_empty(),
                "{example:?} left detail in its title {title:?}: {prefix:?} still reads {:?}",
                again.consumed
            );
        }
    }
}
