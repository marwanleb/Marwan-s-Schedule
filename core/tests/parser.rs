use chrono::NaiveDate;
use ms_core::parse;

fn today() -> NaiveDate {
    // A Monday, fixed so every date-relative case is deterministic.
    NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()
}

#[test]
fn trailing_tokens_are_stripped_from_the_title() {
    let p = parse("math hw fri 5pm ~2h", today());
    assert_eq!(p.title, "math hw");
}

/// The rule that makes the parser usable: scanning stops at the first token it
/// does not recognise, so prose containing date-shaped words survives intact.
#[test]
fn prose_with_a_date_shaped_word_is_left_alone() {
    let p = parse("meet Sarah about the March report", today());
    assert_eq!(p.title, "meet Sarah about the March report");
    assert!(p.consumed.is_empty());
}

#[test]
fn recognised_tokens_yield_their_values() {
    let p = parse("math hw fri 5pm ~2h", today());
    assert_eq!(p.estimate_min, Some(120), "~2h is 120 minutes");
    assert_eq!(p.at, Some(chrono::NaiveTime::from_hms_opt(17, 0, 0).unwrap()), "5pm is 17:00");
    assert_eq!(p.byday, vec!["fri"]);
}

#[test]
fn estimates_accept_minutes_and_fractions() {
    assert_eq!(parse("x ~90m", today()).estimate_min, Some(90));
    assert_eq!(parse("x ~1.5h", today()).estimate_min, Some(90));
    assert_eq!(parse("x ~30m", today()).estimate_min, Some(30));
}

/// Spec 6: capture must never be lost to a syntax mistake.
#[test]
fn parsing_never_fails_and_never_loses_text() {
    for input in [
        "",
        "   ",
        "renew parking",
        "!!!",
        "~",
        "~~~2h",
        ":::",
        "🙂 buy 🙂",
        "a b c d e f g h i j k l m n o p",
        "fri",
        "~2h",
    ] {
        let p = parse(input, today());
        // Whatever is not recognised survives as text. The one deliberate
        // removal is a leading `!!` priority marker.
        let round_trip: Vec<&str> = p
            .title
            .split_whitespace()
            .chain(p.consumed.iter().map(|s| s.as_str()))
            .collect();
        let mut expected: Vec<&str> = input.split_whitespace().collect();
        if p.priority {
            expected[0] = expected[0].trim_start_matches('!');
            expected.retain(|w| !w.is_empty());
        }
        assert_eq!(round_trip, expected, "input {input:?} lost or reordered words");
    }
}

#[test]
fn a_bare_note_keeps_every_word() {
    let p = parse("renew parking", today());
    assert_eq!(p.title, "renew parking");
    assert!(p.consumed.is_empty());
    assert_eq!(p.estimate_min, None);
    assert!(p.byday.is_empty());
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn relative_days_resolve_against_today() {
    // today() is Monday 2026-08-31
    assert_eq!(parse("call mom today", today()).due, Some(ymd(2026, 8, 31)));
    assert_eq!(parse("call mom tomorrow", today()).due, Some(ymd(2026, 9, 1)));
    assert_eq!(parse("call mom tomorrow", today()).title, "call mom");
}

/// The counter-case for the right-to-left rule: a real date word buried in
/// prose must not be eaten.
#[test]
fn a_date_word_inside_prose_is_not_parsed() {
    let p = parse("buy milk for tomorrow's breakfast", today());
    assert_eq!(p.title, "buy milk for tomorrow's breakfast");
    assert_eq!(p.due, None);
}

#[test]
fn a_single_weekday_is_the_next_such_day() {
    assert_eq!(parse("math hw fri", today()).due, Some(ymd(2026, 9, 4)));
    // Today is Monday, so "mon" means today rather than a week away.
    assert_eq!(parse("standup mon", today()).due, Some(ymd(2026, 8, 31)));
}

#[test]
fn several_weekdays_mean_a_repeat_not_a_due_date() {
    let p = parse("gym mon wed fri ~1h", today());
    assert_eq!(p.title, "gym");
    assert_eq!(p.byday, vec!["mon", "wed", "fri"]);
    assert_eq!(p.due, None, "a repeating item has no single due date");
    assert_eq!(p.estimate_min, Some(60));
}

#[test]
fn absolute_dates_parse_in_both_forms() {
    assert_eq!(parse("pay rent 2026-09-15", today()).due, Some(ymd(2026, 9, 15)));

    // Day first, matching the three-part forms. `1/9` is 1 September.
    assert_eq!(parse("pay rent 1/9", today()).due, Some(ymd(2026, 9, 1)));
    // And where the second number cannot be a month, it settles itself.
    assert_eq!(parse("pay rent 12/25", today()).due, Some(ymd(2026, 12, 25)));
}

/// Recurrence is stated, not guessed. One weekday plus `every` repeats; the
/// same weekday without it is a due date.
#[test]
fn every_states_recurrence_explicitly() {
    let weekly = parse("trash every tue 20:00", today());
    assert_eq!(weekly.title, "trash");
    assert!(weekly.repeats, "`every` means it repeats");
    assert_eq!(weekly.byday, vec!["tue"]);
    assert_eq!(weekly.due, None, "a repeat has no single due date");

    let once = parse("math hw fri", today());
    assert!(!once.repeats);
    assert_eq!(once.due, Some(ymd(2026, 9, 4)));
}

#[test]
fn every_works_with_several_weekdays_and_reads_naturally() {
    let p = parse("gym every mon wed fri ~1h", today());
    assert_eq!(p.title, "gym");
    assert!(p.repeats);
    assert_eq!(p.byday, vec!["mon", "wed", "fri"]);
    assert_eq!(p.estimate_min, Some(60));
}

/// `every` only counts as a keyword where tokens are already being read, so
/// prose ending in the word is untouched.
#[test]
fn every_inside_prose_is_not_a_keyword() {
    let p = parse("water the plants every day", today());
    assert_eq!(p.title, "water the plants every day");
    assert!(!p.repeats);
}

#[test]
fn a_time_range_makes_it_a_block_not_a_list_item() {
    let p = parse("MATH210 every mon wed 9:00-10:15", today());
    assert_eq!(p.title, "MATH210");
    assert_eq!(
        p.span,
        Some((
            chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            chrono::NaiveTime::from_hms_opt(10, 15, 0).unwrap()
        ))
    );
    // Typing a span means scheduling, not listing. Spec 3.1.
    assert!(!p.listed, "a block should not clutter the to-do list");
    assert!(p.repeats);
}

#[test]
fn anything_without_a_span_is_listed() {
    assert!(parse("math hw fri ~2h", today()).listed);
    assert!(parse("renew parking", today()).listed);
    // A point time is a to-do with a time, not a block.
    assert!(parse("trash every tue 20:00", today()).listed);
}

#[test]
fn tags_locations_and_priority_are_extracted() {
    let p = parse("problem set fri ~2h #math", today());
    assert_eq!(p.title, "problem set");
    assert_eq!(p.tags, vec!["math"]);
    assert_eq!(p.category, None, "math is not one of the four colour categories");

    let c = parse("swim every tue 18:00-19:00 #body", today());
    assert_eq!(c.category.as_deref(), Some("body"), "a known category colours the item");
    assert_eq!(c.tags, vec!["body"], "and is still a tag");

    let l = parse("MATH210 every mon 9:00-10:15 @Hall 1.204", today());
    assert_eq!(l.title, "MATH210");
    assert_eq!(l.location.as_deref(), Some("Hall 1.204"));

    let bang = parse("!! renew parking", today());
    assert_eq!(bang.title, "renew parking");
    assert!(bang.priority);
}

#[test]
fn daily_repeats_on_every_weekday() {
    let p = parse("water the plants daily", today());
    assert_eq!(p.title, "water the plants");
    assert!(p.repeats);
    assert_eq!(p.byday, vec!["mon", "tue", "wed", "thu", "fri", "sat", "sun"]);
    assert_eq!(p.due, None);

    let v = parse("vitamins daily 08:00", today());
    assert_eq!(v.title, "vitamins");
    assert!(v.repeats);
    assert_eq!(v.at, Some(chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap()));
}

/// `daily` is safe to treat as a keyword because it rarely ends a sentence.
/// The bare word `day` is not, which is why it stays prose.
#[test]
fn daily_does_not_eat_ordinary_sentences() {
    assert_eq!(parse("read the daily news", today()).title, "read the daily news");
    assert_eq!(parse("water the plants every day", today()).title, "water the plants every day");
}

/// Fixed vs floating is stated with a marker and defaults to fixed: a class
/// that follows you to the wrong hour is worse than a gym slot that does.
#[test]
fn items_are_fixed_to_their_zone_unless_told_to_float() {
    assert!(parse("MATH210 every mon 9:00-10:15", today()).pinned, "fixed by default");
    assert!(parse("renew parking", today()).pinned);

    for text in ["wake up daily 08:00 #floating", "wake up daily 08:00 #systime"] {
        let p = parse(text, today());
        assert!(!p.pinned, "{text:?} should follow the machine");
        assert_eq!(p.title, "wake up");
    }

    let explicit = parse("MATH210 every mon 9:00-10:15 #fixed", today());
    assert!(explicit.pinned);
    assert_eq!(explicit.title, "MATH210");
}

/// The markers are consumed, not left lying around as ordinary tags — they
/// describe placement, and a "#floating" chip on the tile would be noise.
#[test]
fn zone_markers_do_not_become_tags() {
    let p = parse("gym every mon wed #floating #body", today());
    assert_eq!(p.tags, vec!["body"], "only the real tag survives");
    assert_eq!(p.category.as_deref(), Some("body"));
    assert!(!p.pinned);
}

/// Real input that failed: full weekday names, a one-letter meridiem, and a
/// room number with a space in it.
#[test]
fn a_real_class_line_parses() {
    let p = parse("PHYS201 every monday wednesday 10:30-12:00p @Hall 2.106", today());

    assert_eq!(p.title, "PHYS201");
    assert!(p.repeats);
    assert_eq!(p.byday, vec!["mon", "wed"]);
    assert_eq!(
        p.span,
        Some((
            chrono::NaiveTime::from_hms_opt(10, 30, 0).unwrap(),
            chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap()
        ))
    );
    assert_eq!(p.location.as_deref(), Some("Hall 2.106"));
}

#[test]
fn weekdays_are_accepted_written_out_in_full() {
    for (text, code) in [
        ("x every monday", "mon"), ("x every tuesday", "tue"),
        ("x every wednesday", "wed"), ("x every thursday", "thu"),
        ("x every friday", "fri"), ("x every saturday", "sat"),
        ("x every sunday", "sun"),
    ] {
        let p = parse(text, today());
        assert_eq!(p.byday, vec![code], "{text:?}");
        assert_eq!(p.title, "x", "{text:?}");
    }
    // Mixed forms in one line collapse to the same day, not two.
    assert_eq!(parse("x every mon monday", today()).byday, vec!["mon"]);
}

#[test]
fn a_single_letter_meridiem_is_a_time() {
    let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
    assert_eq!(parse("x 12:00p", today()).at, Some(t(12, 0)));
    assert_eq!(parse("x 5p", today()).at, Some(t(17, 0)));
    assert_eq!(parse("x 9a", today()).at, Some(t(9, 0)));
    assert_eq!(parse("x 9:30a-11a", today()).span, Some((t(9, 30), t(11, 0))));
}

/// `@` runs to the end of the line, because room numbers and building names
/// contain spaces. It is the last thing on a line in practice.
#[test]
fn a_location_may_contain_spaces() {
    let p = parse("lecture every mon 9:00-10:00 @Hall 2.106", today());
    assert_eq!(p.title, "lecture");
    assert_eq!(p.location.as_deref(), Some("Hall 2.106"));
    assert_eq!(p.byday, vec!["mon"]);

    let bare = parse("coffee @the corner cafe", today());
    assert_eq!(bare.title, "coffee");
    assert_eq!(bare.location.as_deref(), Some("the corner cafe"));
}

/// An email address is not a location: `@` only starts one at a word boundary.
#[test]
fn an_email_address_is_left_alone() {
    let p = parse("email bob@example.com about the lease", today());
    assert_eq!(p.title, "email bob@example.com about the lease");
    assert_eq!(p.location, None);
}

/// A tag after a location must survive: `@` covers the place, not the rest of
/// the line regardless of what follows.
#[test]
fn a_tag_after_a_location_is_still_a_tag() {
    let p = parse("PHYS201 every mon 10:30-12:00 @Hall 2.106 #work", today());
    assert_eq!(p.title, "PHYS201");
    assert_eq!(p.location.as_deref(), Some("Hall 2.106"));
    assert_eq!(p.tags, vec!["work"]);
    assert_eq!(p.category.as_deref(), Some("work"));
    assert_eq!(p.byday, vec!["mon"]);
}

/// Real input that failed: a meridiem written as its own word, and a
/// three-part date.
#[test]
fn an_airport_pickup_parses() {
    let p = parse("Pick up a friend from the airport 9-02-2026 8:00 pm", today());

    assert_eq!(p.title, "Pick up a friend from the airport");
    assert_eq!(p.due, Some(ymd(2026, 2, 9)), "day-month-year");
    assert_eq!(p.at, Some(chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap()));
}

#[test]
fn a_meridiem_may_be_its_own_word() {
    let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
    assert_eq!(parse("x 8:00 pm", today()).at, Some(t(20, 0)));
    assert_eq!(parse("x 8 pm", today()).at, Some(t(20, 0)));
    assert_eq!(parse("x 9:30 am", today()).at, Some(t(9, 30)));
    // Both words are eaten, not just the marker.
    assert_eq!(parse("x 8:00 pm", today()).title, "x");
}

/// Three-part dates. Where one number cannot be a month it settles itself;
/// where both could be, day comes first.
#[test]
fn three_part_dates_resolve_sensibly() {
    assert_eq!(parse("x 9-02-2026", today()).due, Some(ymd(2026, 2, 9)), "day first");
    assert_eq!(parse("x 2/9/2026", today()).due, Some(ymd(2026, 9, 2)), "slashes too");
    assert_eq!(parse("x 25-12-2026", today()).due, Some(ymd(2026, 12, 25)), "25 can only be a day");
    assert_eq!(parse("x 12/25/2026", today()).due, Some(ymd(2026, 12, 25)), "25 can only be a month here");
    assert_eq!(parse("x 2026-09-02", today()).due, Some(ymd(2026, 9, 2)), "ISO still wins");
    // Not a date at all.
    assert_eq!(parse("x 45-99-2026", today()).title, "x 45-99-2026");
}

/// A version number is not a date: three parts only count when the last is a
/// four-digit year.
#[test]
fn a_version_number_is_not_a_date() {
    assert_eq!(parse("upgrade to 1-2-3", today()).title, "upgrade to 1-2-3");
    assert_eq!(parse("bump rustc to 1.95.0", today()).title, "bump rustc to 1.95.0");
}

// ── found by tools/parser_sweep.py, not by hand ──────────────────────────
// The sweep enumerates date and time formats mechanically and checks each
// against `dateutil`. Everything below is a form that a mature parser accepts
// and this one silently dropped into the title.

#[test]
fn month_names_are_dates() {
    let sep2 = ymd(2026, 9, 2);
    for text in [
        "x 2 Sep", "x 2 sep", "x 2 SEP", "x 2 September", "x 2 september",
        "x Sep 2", "x September 2", "x sep 2",
        "x 2 Sep 2026", "x Sep 2 2026", "x Sep 2, 2026", "x September 2, 2026",
        "x 2nd September", "x September 2nd",
    ] {
        let p = parse(text, today());
        assert_eq!(p.due, Some(sep2), "{text:?}");
        assert_eq!(p.title, "x", "{text:?} left something in the title");
    }
}

/// A month name on its own is not a date — there is no day in it, and
/// guessing one would mangle "finish the essay in March".
#[test]
fn a_bare_month_name_is_not_a_date() {
    assert_eq!(parse("finish the essay in March", today()).due, None);
    assert_eq!(parse("finish the essay in March", today()).title, "finish the essay in March");
    assert_eq!(parse("x September", today()).title, "x September");
}

#[test]
fn noon_and_midnight_are_times() {
    let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
    assert_eq!(parse("lunch noon", today()).at, Some(t(12)));
    assert_eq!(parse("lunch midday", today()).at, Some(t(12)));
    assert_eq!(parse("deadline midnight", today()).at, Some(t(0)));
    assert_eq!(parse("lunch noon", today()).title, "lunch");
}

#[test]
fn dots_separate_dates_too() {
    // How dates are written across most of Europe, where he spends months.
    assert_eq!(parse("x 02.09.2026", today()).due, Some(ymd(2026, 9, 2)));
    assert_eq!(parse("x 2.9.2026", today()).due, Some(ymd(2026, 9, 2)));
}

#[test]
fn two_digit_years_are_understood() {
    assert_eq!(parse("x 02/09/26", today()).due, Some(ymd(2026, 9, 2)));
    assert_eq!(parse("x 02-09-26", today()).due, Some(ymd(2026, 9, 2)));
}

#[test]
fn more_time_spellings() {
    let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
    assert_eq!(parse("x 8 p.m.", today()).at, Some(t(20, 0)));
    assert_eq!(parse("x 8p.m.", today()).at, Some(t(20, 0)));
    assert_eq!(parse("x 20:00:00", today()).at, Some(t(20, 0)));
}

/// Still not dates, and must stay out of the title.
#[test]
fn the_sweep_cases_that_should_keep_failing() {
    for text in [
        "upgrade to 1-2-3",
        "bump rustc to 1.95.0",
        "read chapters 2-9",
        "call him on 5",
    ] {
        let p = parse(text, today());
        assert_eq!(p.title, text, "{text:?} should be left whole");
    }
}

// ── found by a black-box tester that had not seen the code ───────────────

/// "at", "on", "by", "due" sit between a task and its time constantly. They
/// used to halt the scan, which threw away the date behind them as well.
#[test]
fn filler_words_do_not_halt_the_scan() {
    let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();

    let p = parse("flight to paris on sept 3 at 6:45am", today());
    assert_eq!(p.title, "flight to paris");
    assert_eq!(p.due, Some(ymd(2026, 9, 3)));
    assert_eq!(p.at, Some(t(6, 45)));

    let q = parse("lab report due wed by 11:59 pm", today());
    assert_eq!(q.title, "lab report");
    assert_eq!(q.at, Some(t(23, 59)));

    let r = parse("lunch at noon monday", today());
    assert_eq!(r.title, "lunch");
    assert_eq!(r.at, Some(t(12, 0)));
}

/// A filler word on its own is still just a word.
#[test]
fn a_filler_word_alone_is_not_stripped() {
    assert_eq!(parse("think about it", today()).title, "think about it");
    assert_eq!(parse("the thing to do", today()).title, "the thing to do");
}

#[test]
fn common_weekday_abbreviations_work() {
    for (text, code) in [
        ("x tues", "tue"), ("x thurs", "thu"), ("x weds", "wed"),
        ("x tue", "tue"), ("x thur", "thu"),
    ] {
        assert_eq!(parse(text, today()).byday, vec![code], "{text:?}");
    }
}

/// "mondays" reads as a repeat, which is what the plural means.
#[test]
fn a_plural_weekday_means_every_week() {
    let p = parse("team meeting mondays at 3pm", today());
    assert_eq!(p.title, "team meeting");
    assert_eq!(p.byday, vec!["mon"]);
    assert!(p.repeats, "the plural is the recurrence");
}

#[test]
fn weekdays_may_be_separated_by_slashes() {
    let p = parse("gym mon/wed/fri 6am", today());
    assert_eq!(p.title, "gym");
    assert_eq!(p.byday, vec!["mon", "wed", "fri"]);
    assert!(p.repeats);
}

/// `@` with a space after it is still a location marker.
#[test]
fn an_at_sign_may_stand_alone() {
    let p = parse("meet bob @ starbucks", today());
    assert_eq!(p.title, "meet bob");
    assert_eq!(p.location.as_deref(), Some("starbucks"));
}

/// A location must not swallow a time that follows it.
#[test]
fn a_location_stops_at_a_time() {
    let p = parse("meet bob @starbucks 3pm", today());
    assert_eq!(p.title, "meet bob");
    assert_eq!(p.location.as_deref(), Some("starbucks"));
    assert_eq!(p.at, Some(chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap()));
}

/// "due" and "on" say what kind of thing this is: something to finish by a
/// time, or something that happens at one.
#[test]
fn due_makes_a_task_and_on_makes_a_block_you_can_still_tick() {
    let essay = parse("essay due friday", today());
    assert_eq!(essay.title, "essay");
    assert_eq!(essay.due, Some(ymd(2026, 9, 4)));
    assert!(essay.listed, "a deadline belongs in the list");
    assert!(!essay.scheduled);

    let meeting = parse("standup on friday 9am", today());
    assert_eq!(meeting.title, "standup");
    assert_eq!(meeting.due, Some(ymd(2026, 9, 4)));
    assert!(meeting.scheduled, "it happens at a time, so put it on the week");
    assert!(meeting.listed, "and it is still a thing to finish, so keep it tickable");

    // A bare range is a block and nothing else: a class is not a to-do.
    let class = parse("MATH210 every mon 9:00-10:15", today());
    assert!(!class.listed, "a timetable does not belong in the list");
}

/// "due" wins when both appear — the deadline is the point.
#[test]
fn due_beats_on() {
    let p = parse("lab report due wed by 11:59 pm", today());
    assert!(p.listed);
    assert!(!p.scheduled);
}

/// Without a time there is nothing to place, and an item that is neither
/// listed nor on the week would simply vanish.
#[test]
fn on_without_a_time_stays_in_the_list() {
    let p = parse("haircut on friday", today());
    assert_eq!(p.title, "haircut");
    assert_eq!(p.due, Some(ymd(2026, 9, 4)));
    assert!(!p.scheduled, "nothing to place it at");
    assert!(p.listed, "so it must still be somewhere");
}

/// The existing rule is unchanged: an explicit span is a block either way.
#[test]
fn a_span_is_still_a_block_without_the_keyword() {
    let p = parse("MATH210 every mon 9:00-10:15", today());
    assert!(!p.listed);
}

/// "1 oct 9am" once read as October 9th with no time: the day check accepted
/// any trailing letters as an ordinal, so "9am" counted as the 9th.
#[test]
fn a_time_after_a_written_date_is_a_time_not_a_day() {
    let hhmm = |p: &ms_core::Parsed| p.at.map(|t| t.format("%H:%M").to_string());
    for line in ["dentist 1 oct 9am", "dentist oct 1 9am"] {
        let p = parse(line, today());
        assert_eq!(p.title, "dentist", "{line:?}");
        assert_eq!(p.due, Some(ymd(2026, 10, 1)), "{line:?}");
        assert_eq!(hhmm(&p), Some("09:00".into()), "{line:?}");
    }
    // Ordinals and the comma before a year still read as days.
    assert_eq!(parse("x 2nd September", today()).due, Some(ymd(2026, 9, 2)));
    assert_eq!(parse("x September 2, 2026", today()).due, Some(ymd(2026, 9, 2)));
}

/// A filler at the very end has nothing to introduce, so it is part of the
/// sentence. Eating it lost the word: "meet him by" became "meet him".
#[test]
fn a_trailing_filler_word_is_kept() {
    for line in ["meet him by", "email the notes I got from", "x at", "essay due", "the talk is on"] {
        let p = parse(line, today());
        assert_eq!(p.title, line, "nothing may be lost from {line:?}");
        assert!(p.consumed.is_empty(), "{line:?} consumed {:?}", p.consumed);
    }
    // Between a title and its details a filler still does its job.
    assert_eq!(parse("lunch at noon monday", today()).title, "lunch");
    assert_eq!(parse("essay due friday", today()).title, "essay");
}

/// A range on a date places a one-off block there and stays out of the list.
#[test]
fn a_bare_range_on_a_date_is_scheduled_not_listed() {
    let span = Some((
        chrono::NaiveTime::from_hms_opt(11, 20, 0).unwrap(),
        chrono::NaiveTime::from_hms_opt(12, 20, 0).unwrap(),
    ));
    for (line, due) in [
        ("Appt 1 oct 11:20-12:20", (2026, 10, 1)),
        ("Appt 2026-10-01 11:20-12:20", (2026, 10, 1)),
        ("Appt thu 11:20-12:20p", (2026, 9, 3)),
    ] {
        let p = parse(line, today());
        assert_eq!(p.title, "Appt", "{line}");
        assert_eq!(p.span, span, "{line}");
        assert_eq!(p.due, NaiveDate::from_ymd_opt(due.0, due.1, due.2), "{line}");
        assert!(p.scheduled, "{line}");
        assert!(!p.listed, "{line}");
    }
}

/// "on" with a range still does both: on the week and tickable.
#[test]
fn on_with_a_range_is_scheduled_and_listed() {
    let p = parse("Appt on 1 oct 11:20am-12:20pm", today());
    assert!(p.scheduled);
    assert!(p.listed);
}

/// Only a dated one-off becomes a placed block. A weekly range is a rule, and
/// "due" says deadline.
#[test]
fn a_range_without_a_single_date_is_not_placed() {
    assert!(!parse("MATH210 every mon 9:00-10:15", today()).scheduled);
    assert!(!parse("x 9:00-10:15", today()).scheduled);
    assert!(!parse("essay due fri 9:00-10:15", today()).scheduled);
}
