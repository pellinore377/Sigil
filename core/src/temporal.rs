pub fn preview(input: &str) -> String {
    if input.len() > 512 {
        return String::new();
    }
    let fields = input.splitn(5, '\n').collect::<Vec<_>>();
    if fields.len() != 5 || fields[2].len() > 64 || fields[4].contains(['\n', '\r']) {
        return String::new();
    }
    let Ok(now) = fields[1].parse::<u64>() else {
        return String::new();
    };
    if now == 0 || now > i64::MAX as u64 {
        return String::new();
    }
    let order = match fields[3] {
        "month" => sigil_text::time::DateOrder::MonthFirst,
        "day" => sigil_text::time::DateOrder::DayFirst,
        _ => return String::new(),
    };
    match fields[0] {
        "Timer" => sigil_text::time::duration(fields[4])
            .and_then(|seconds| sigil_text::time::Timer::new(now, seconds).map(|_| seconds))
            .map(|seconds| format!("{seconds}s\n{seconds}\n"))
            .unwrap_or_default(),
        "Reminder" => sigil_text::time::date_confirmation(fields[4], now, fields[2], Some(order))
            .map(|(at, source, offset)| format!("{source}\n{at}\n{offset}"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmed_dates_survive_midnight_locale_and_dst_without_reinterpretation() {
        for (input, expected, offset) in [
            ("tomorrow 2:30am", "2026-03-08T03:30:00", "-04:00"),
            ("2026-11-01 1:30am", "2026-11-01T01:30:00", "-04:00"),
            ("07/05/27 9:30am", "2027-07-05T09:30:00", "-04:00"),
        ] {
            let result = preview(&format!(
                "Reminder\n1772945999\nAmerica/New_York\nmonth\n{input}"
            ));
            let parts = result.split('\n').collect::<Vec<_>>();
            assert_eq!(parts[0], expected);
            assert_eq!(parts[2], offset);
            let reparsed =
                sigil_text::time::resolve_date(parts[0], 1773000000, "America/New_York", None)
                    .unwrap();
            assert_eq!(reparsed.to_string(), parts[1]);
        }
        assert!(preview("Reminder\n1772945999\nUTC\nday\n07/05/27 9:30am")
            .starts_with("2027-05-07T09:30:00\n"));
        assert_eq!(
            preview("Timer\n1772945999\nUTC\nmonth\n1m 30s"),
            "90s\n90\n"
        );
    }
    #[test]
    fn invalid_previews_do_not_echo_source_or_accept_partial_input() {
        for input in [
            "",
            "Timer\n0\nUTC\nmonth\n5m",
            "Timer\n1772945999\nUTC\nmonth\n0s",
            "Reminder\n1772945999\nBad/Zone\nmonth\ntomorrow",
            "Reminder\n1772945999\nUTC\nother\ntomorrow",
            "Timer\n1772945999\nUTC\nmonth\n5m\nSECRET",
            "Timer\n1772945999\nUTC\nmonth\nSECRET",
        ] {
            assert!(preview(input).is_empty());
        }
        assert!(preview(&"x".repeat(513)).is_empty());
    }
}
