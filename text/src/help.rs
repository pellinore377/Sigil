use crate::{Error, Limits, Text};

pub struct Topic {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub category: &'static str,
    pub description: &'static str,
    pub template: &'static str,
    pub options: &'static [&'static str],
    pub example: &'static str,
    pub structured: bool,
}
macro_rules! topic {
    ($name:literal, $category:literal, $description:literal, $template:literal, [$($option:literal),*], $example:literal, $structured:literal) => {
        Topic { name: $name, aliases: &[], category: $category, description: $description,
            template: $template, options: &[$($option),*], example: $example, structured: $structured }
    };
}
pub const TOPICS: &[Topic] = &[
    topic!(
        "info", "info", "Use modifier::content; to format text. Stack modifiers together. Inline formatting can end at a line break without the final semicolon.",
        "modifier::content;", [], "bold::blue::Hello;\ntimer::5m;\nremind::tomorrow 9am::Call;", false
    ),
    topic!(
        "markdown",
        "text",
        "Headings, lists, quotes, links and code",
        "# Heading\n\nText",
        [],
        "**bold** and `code`",
        false
    ),
    topic!(
        "bold",
        "text",
        "Bold text",
        "bold::<text>;",
        [],
        "bold::Hello;",
        false
    ),
    topic!(
        "italic",
        "text",
        "Italic text",
        "italic::<text>;",
        [],
        "italic::Hello;",
        false
    ),
    topic!(
        "strike",
        "text",
        "Struck-through text",
        "strike::<text>;",
        [],
        "strike::old;",
        false
    ),
    topic!(
        "underline",
        "text",
        "Underlined text",
        "underline::<text>;",
        [],
        "underline::Hello;",
        false
    ),
    topic!(
        "mono",
        "text",
        "Monospaced text",
        "mono::<text>;",
        [],
        "mono::status;",
        false
    ),
    topic!(
        "mark",
        "text",
        "Highlighted text",
        "mark::<text>;",
        [],
        "mark::Remember;",
        false
    ),
    topic!(
        "spoiler",
        "text",
        "Tap to reveal",
        "spoiler::<text>;",
        [],
        "spoiler::Surprise;",
        false
    ),
    topic!(
        "scratch",
        "text",
        "Scratch to reveal",
        "scratch::<text>;",
        [],
        "scratch::Surprise;",
        false
    ),
    topic!(
        "redact",
        "text",
        "Permanently remove source before sending",
        "redact::<text>;",
        [],
        "redact::private;",
        false
    ),
    topic!(
        "big",
        "text",
        "Larger text",
        "big[1-3]::<text>;",
        ["1", "2", "3"],
        "big2::Hello;",
        false
    ),
    topic!(
        "small",
        "text",
        "Smaller text",
        "small[1-3]::<text>;",
        ["1", "2", "3"],
        "small2::Aside;",
        false
    ),
    topic!(
        "color",
        "colors",
        "Theme-resolved color and shade",
        "<hue>[1-3]::<text>;",
        ["red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray"],
        "blue2::Hello;",
        false
    ),
    topic!(
        "gradient",
        "colors",
        "Multiple theme colors",
        "<hue>-<hue>::<text>;",
        ["2–9 stops"],
        "red-blue::Hello;",
        false
    ),
    topic!(
        "rainbow",
        "colors",
        "Theme-resolved rainbow",
        "rainbow::<text>;",
        [],
        "rainbow::Hello;",
        false
    ),
    topic!(
        "shake",
        "animations",
        "Brief horizontal shake",
        "shake::<text>;",
        [],
        "shake::Whoa;",
        false
    ),
    topic!(
        "wave",
        "animations",
        "Glyph wave",
        "wave::<text>;",
        [],
        "wave::Hello;",
        false
    ),
    topic!(
        "pulse",
        "animations",
        "Two gentle pulses",
        "pulse::<text>;",
        [],
        "pulse::Hello;",
        false
    ),
    topic!(
        "glow",
        "animations",
        "Brief glow",
        "glow::<text>;",
        [],
        "glow::Hello;",
        false
    ),
    topic!(
        "typewriter",
        "animations",
        "One-pass text reveal",
        "typewriter::<text>;",
        [],
        "typewriter::Hello;",
        false
    ),
    topic!(
        "sparkle",
        "animations",
        "Brief particle burst",
        "sparkle::<text>;",
        [],
        "sparkle::Hello;",
        false
    ),
    topic!(
        "glitch",
        "animations",
        "Brief glyph substitution",
        "glitch::<text>;",
        [],
        "glitch::Hello;",
        false
    ),
    topic!(
        "scatter",
        "animations",
        "Glyphs settle into place",
        "scatter::<text>;",
        [],
        "scatter::Hello;",
        false
    ),
    topic!(
        "flip",
        "animations",
        "Glyph flip",
        "flip::<text>;",
        [],
        "flip::Hello;",
        false
    ),
    topic!(
        "barrel",
        "animations",
        "One full rotation",
        "barrel::<text>;",
        [],
        "barrel::Hello;",
        false
    ),
    topic!(
        "@",
        "text",
        "Select a known identity; ambiguous names require a choice",
        "@<user>[:<server>]",
        [],
        "@user:example.org",
        false
    ),
    topic!(
        "@::",
        "other",
        "Open the contact picker",
        "@::",
        [],
        "@::",
        true
    ),
    topic!(
        "checklist",
        "lists",
        "Shared checklist, task or recurring list",
        "checklist::[mode::]<title>\n- <item>;",
        ["task", "weekly", "monthly", "yearly", "-x-", "-r-"],
        "checklist::Shopping\n- Milk;",
        true
    ),
    topic!(
        "poll",
        "lists",
        "Single or multiple choices",
        "poll::[options::]<question>\n- <choice>\n- <choice>;",
        ["open", "closed", "multi", "multiN"],
        "poll::Lunch?\n- Soup\n- Salad;",
        true
    ),
    topic!(
        "note",
        "lists",
        "Conversation note",
        "note::<text>;",
        [],
        "note::Bring water;",
        true
    ),
    topic!(
        "remind",
        "time",
        "Resolve a reminder in the sender's timezone",
        "remind::<date>::<text>;",
        ["ISO date", "tomorrow", "next week", "weekday"],
        "remind::tomorrow 9:00::Call;",
        true
    ),
    topic!(
        "timer",
        "time",
        "Fixed start and end time",
        "timer::<duration>;",
        ["d", "h", "m", "s"],
        "timer::5m;",
        true
    ),
    topic!(
        "countdown",
        "time",
        "Time until a fixed date",
        "countdown::<date>::<label>;",
        [],
        "countdown::2027-01-01::New year;",
        true
    ),
    topic!(
        "ago",
        "time",
        "Time since a fixed date",
        "ago::<date>::<label>;",
        [],
        "ago::2026-01-01::Started;",
        true
    ),
    topic!(
        "chart",
        "data",
        "Finite numeric data",
        "chart::<kind>::<title>\n- <label>=<value>;",
        ["pie", "donut", "bar", "line", "area", "scatter"],
        "chart::pie::Share\n- A=1\n- B=2;",
        true
    ),
    topic!(
        "diagram",
        "diagrams",
        "Bounded graph data",
        "diagram::<kind>::<title>\n- <node> -> <node>;",
        ["flow", "sequence", "timeline", "mindmap", "org", "state"],
        "diagram::flow::Plan\n- Start -> End;",
        true
    ),
    topic!(
        "table",
        "data",
        "Structured rows and columns",
        "table::<column>|<column>\n- <cell>|<cell>;",
        [],
        "table::Name|Age\n- Lee|30;",
        true
    ),
    topic!(
        "recipe",
        "other",
        "Ingredients and ordered steps",
        "recipe::<title>\ningredients:\n- <item>\nsteps:\n- <step>;",
        ["serves", "time"],
        "recipe::Tea\ningredients:\n- Water\nsteps:\n- Boil;",
        true
    ),
    topic!(
        "calc",
        "utility",
        "Bounded arithmetic, never code execution",
        "calc::<expression>;",
        ["+", "-", "*", "/", "%", "^", "()"],
        "calc::17*34;",
        true
    ),
    topic!(
        "convert",
        "utility",
        "Unit conversion with stored result",
        "convert::<value><unit>;",
        ["temperature", "distance", "weight", "volume", "speed"],
        "convert::5mi;",
        true
    ),
    topic!(
        "math",
        "other",
        "Literal mathematical expression",
        "math::<expression>;",
        [],
        "math::x^2;",
        true
    ),
    topic!(
        "art",
        "other",
        "Whitespace-preserving literal art",
        "art::\n<art>\n;",
        [],
        "art::\n /\\_/\\\n;",
        true
    ),
    topic!(
        "qr",
        "utility",
        "Typed QR payload",
        "qr::<kind>::<value>;",
        ["url", "text", "wifi", "contact"],
        "qr::text::Hello;",
        true
    ),
    topic!(
        "roll",
        "utility",
        "Resolve dice once before sending",
        "roll::<count>d<sides>;",
        [],
        "roll::2d6;",
        true
    ),
    topic!(
        "pick",
        "utility",
        "Resolve a choice once before sending",
        "pick::<option>, <option>;",
        [
            "number",
            "flip",
            "food",
            "movie",
            "book",
            "activity",
            "chore",
            "meal",
            "color",
            "direction",
            "yesno"
        ],
        "pick::Tea, Coffee;",
        true
    ),
    topic!(
        "kbd",
        "utility",
        "Key sequence",
        "kbd::<key>+<key>;",
        [],
        "kbd::Ctrl+S;",
        true
    ),
    topic!(
        "rate",
        "utility",
        "Rating",
        "rate::<score>/<maximum>;",
        [],
        "rate::4/5;",
        true
    ),
    topic!(
        "progress",
        "utility",
        "Clamped progress percentage",
        "progress::<percent>;",
        [],
        "progress::75;",
        true
    ),
    topic!(
        "quote",
        "utility",
        "Attributed quotation",
        "quote::<author>::[source::]<text>;",
        [],
        "quote::Lee::Hello;",
        true
    ),
    topic!(
        "swatch",
        "utility",
        "Explicit color sample",
        "swatch::<color>;",
        ["hex", "rgb", "rgba", "hsl"],
        "swatch::#336699;",
        true
    ),
    topic!(
        "translate",
        "other",
        "Resolve through the configured translation provider",
        "translate::<language>::<text>;",
        ["auto"],
        "translate::es::Hello;",
        false
    ),
    topic!(
        "define",
        "other",
        "Configured dictionary snapshot",
        "define::<word>;",
        [],
        "define::petrichor;",
        false
    ),
    topic!(
        "weather",
        "other",
        "Confirmed place and configured weather snapshot",
        "weather::<place>::[forecast];",
        ["forecast"],
        "weather::Seattle;",
        false
    ),
    topic!(
        "help",
        "info",
        "Local help or a shareable cheat sheet",
        "help::<topic or category>;",
        [],
        "help::info;",
        false
    ),
];
pub fn search(query: &str) -> impl Iterator<Item = &'static Topic> + '_ {
    TOPICS.iter().filter(move |topic| {
        query.is_empty()
            || topic.name.contains(query)
            || topic.category.contains(query)
            || topic.aliases.iter().any(|alias| alias.contains(query))
    })
}
pub fn catalog(query: &str) -> String {
    if query.len() > 128 {
        return String::new();
    }
    let words = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    TOPICS
        .iter()
        .filter(|topic| {
            let haystack = format!(
                "{} {} {} {} {} {}",
                topic.name,
                topic.category,
                topic.description,
                topic.template,
                topic.options.join(" "),
                topic.aliases.join(" ")
            )
            .to_lowercase();
            words.iter().all(|word| haystack.contains(word))
        })
        .map(|topic| {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                topic.name,
                topic.category,
                topic.description,
                topic.template,
                topic.options.join("\n"),
                topic.example
            )
        })
        .collect::<Vec<_>>()
        .join("\u{1e}")
}
pub(crate) fn structured_prefix(source: &str) -> bool {
    TOPICS.iter().any(|topic| {
        topic.structured
            && ((topic.name == "@::" && source.starts_with("@::"))
                || source
                    .strip_prefix(topic.name)
                    .is_some_and(|tail| tail.starts_with("::")))
    })
}
pub fn sheet(query: &str, limits: Limits) -> Result<Text, Error> {
    if query.len() > 64 {
        return Err(Error::Limit);
    }
    if query == "info" {
        return Text::plain("SigilText\nUse modifier::content; and stack bold::blue::Hello;. Inline formatting may run to the end of a line without a semicolon.\nTry timer::5m; or remind::tomorrow 9:00::Call;.\nType help:: for the full reference.", limits);
    }
    let mut rows = Vec::new();
    for topic in search(query) {
        rows.push(format!(
            "{} — {}\n{}\nExample: {}",
            topic.name, topic.description, topic.template, topic.example
        ));
    }
    if rows.is_empty() {
        return Err(Error::Invalid);
    }
    Text::plain(&rows.join("\n\n"), limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_reference_filters_options_and_descriptions_without_changing_examples() {
        let result = catalog("");
        assert_eq!(result.split('\u{1e}').count(), TOPICS.len());
        for (row, topic) in result.split('\u{1e}').zip(TOPICS) {
            let fields = row.split('\u{1f}').collect::<Vec<_>>();
            assert_eq!(fields.len(), 6);
            assert_eq!(fields[5], topic.example);
        }
        assert!(catalog("WAVE").starts_with("wave\u{1f}"));
        assert!(catalog("number").contains("pick\u{1f}"));
        assert!(catalog("animations horizontal").starts_with("shake\u{1f}"));
        assert!(catalog("NONEXISTENT_TOPIC").is_empty());
        assert!(catalog(&"a".repeat(129)).is_empty());
    }
    #[test]
    fn help_examples_use_the_same_parser_as_the_composer() {
        let origin = crate::Origin {
            message: [1; 32],
            creator: [2; 32],
            created_at: 1800000000,
            timezone: Some("UTC"),
        };
        for topic in TOPICS
            .iter()
            .filter(|topic| topic.structured && topic.name != "@::")
        {
            let draft = crate::parse_card(topic.example, origin, Default::default()).unwrap();
            assert!(
                matches!(draft.content, crate::Parsed::Card(_)),
                "{}: {}",
                topic.name,
                topic.example
            );
        }
        for category in [
            "info",
            "text",
            "colors",
            "animations",
            "lists",
            "time",
            "utility",
            "data",
            "diagrams",
            "other",
        ] {
            let sheet = sheet(category, Limits::default()).unwrap();
            assert!(Text::from_bytes(&sheet.to_bytes().unwrap()).is_ok());
        }
    }
}
