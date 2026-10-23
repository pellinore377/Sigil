
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
        "assemble",
        "animations",
        "Every letter finds its place",
        "assemble::<text>;",
        [],
        "assemble::Hello;",
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
        "Rating, or a request for one out of 5 or 10",
        "rate::<score>/<maximum>; or rate::[closed::]<5|10>::<question>;",
        [],
        "rate::5::How was dinner?;",
        true
    ),
    topic!(
        "progress",
        "utility",
        "Clamped progress percentage with an optional title",
        "progress::<percent>[::<title>];",
        [],
        "progress::75::Sprint burndown;",
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
/// A cheat sheet as the card shows it: a title, an optional lead, then labelled copyable examples.
pub struct Sheet {
    pub title: String,
    pub intro: Option<String>,
    pub entries: Vec<(&'static str, &'static str)>,
    pub more: usize,
}
const CATEGORIES: &[(&str, &str, &str)] = &[
    ("text", "Text", "Markdown, modifiers, sizes, spoilers and marks."),
    ("colors", "Colors", "Hues, shades, gradients and rainbow."),
    ("animations", "Animations", "Short effects that play once as the message arrives."),
    ("lists", "Lists", "Checklists, polls and notes."),
    ("time", "Time", "Reminders, timers, countdowns and elapsed time."),
    ("utility", "Utility", "Calculations, conversions, randomizers, QR codes and small helpers."),
    ("data", "Data", "Charts and tables."),
    ("diagrams", "Diagrams", "Flowcharts, sequences, timelines, mind maps, org charts and state machines."),
    ("other", "Other", "Recipes, math, art, translation, definitions and weather."),
];
const SHEET_ROWS: usize = 7;
/// The name a learner knows a construct by, as the reference labels its cheats.
pub fn label(name: &str) -> &'static str {
    match name {
        "info" => "Getting started", "markdown" => "Markdown", "bold" => "Bold", "italic" => "Italic", "strike" => "Strikethrough",
        "underline" => "Underline", "mono" => "Monospace", "mark" => "Highlight", "spoiler" => "Spoiler", "scratch" => "Scratch-off",
        "redact" => "Redaction", "big" => "Larger text", "small" => "Smaller text", "color" => "Hue and shade", "gradient" => "Gradient",
        "rainbow" => "Rainbow", "shake" => "Shake", "wave" => "Wave", "pulse" => "Pulse", "glow" => "Glow", "typewriter" => "Typewriter",
        "sparkle" => "Sparkle", "glitch" => "Glitch", "scatter" => "Scatter", "flip" => "Flip", "barrel" => "Barrel roll",
        "assemble" => "Assemble", "@" => "Mention", "@::" => "Contact picker", "checklist" => "Checklist", "poll" => "Poll",
        "note" => "Note", "remind" => "Reminder", "timer" => "Timer", "countdown" => "Countdown", "ago" => "Elapsed time",
        "chart" => "Chart", "diagram" => "Diagram", "table" => "Table", "recipe" => "Recipe", "calc" => "Calculation",
        "convert" => "Conversion", "math" => "Math", "art" => "ASCII art", "qr" => "QR code", "roll" => "Dice roll", "pick" => "Pick",
        "kbd" => "Keyboard shortcut", "rate" => "Rating", "progress" => "Progress", "quote" => "Pull quote", "swatch" => "Color swatch",
        "translate" => "Translation", "define" => "Definition", "weather" => "Weather", "help" => "Help", _ => "Example",
    }
}
/// Further copyable examples per topic: (topic, row label, source). The topic's own example is labelled by `lead`.
const EXAMPLES: &[(&str, &str, &str)] = &[
    ("color", "Lighter shade", "green1::Hello;"),
    ("gradient", "Three stops", "red-orange-yellow::Hello;"),
    ("checklist", "Recurring checklist", "checklist::weekly::The weekly reset\n-r- Water the plants\n- Buy a notebook;"),
    ("checklist", "Task list", "checklist::task::Ready to publish\n- Review the copy\n- Publish;"),
    ("poll", "Choose several", "poll::multi::What should we bring?\n- A camera\n- Coffee;"),
    ("timer", "Longer timer", "timer::1h30m;"),
    ("countdown", "Countdown to a time", "countdown::2027-01-01 18:00::Doors open;"),
    ("ago", "Since yesterday", "ago::yesterday::A little progress;"),
    ("chart", "Bar chart", "chart::bar::Week\n- Mon=12\n- Tue=20\n- Wed=17;"),
    ("chart", "Line chart", "chart::line::The week\n- Mon=12\n- Tue=20\n- Wed=14;"),
    ("diagram", "Sequence", "diagram::sequence::Call\n- Client -> Server: hello\n- Server --> Client: ready;"),
    ("diagram", "Timeline", "diagram::timeline::Project\n- 2026-03 = Started\n- 2026-09 = Launch;"),
    ("diagram", "Mind map", "diagram::mindmap::Trip\n- Trip -> Packing\n- Trip -> Route;"),
    ("diagram", "Org chart", "diagram::org::Team\n- Lead -> Design\n- Lead -> Engineering;"),
    ("diagram", "State machine", "diagram::state::Call\n- Idle -> Ringing [invite]\n- Ringing -> Connected [answer];"),
];
fn lead(topic: &Topic) -> &'static str {
    match topic.name {
        "chart" => "Pie chart",
        "diagram" => "Flowchart",
        name => label(name),
    }
}
fn examples(name: &'static str) -> impl Iterator<Item = (&'static str, &'static str)> {
    EXAMPLES.iter().filter(move |(topic, ..)| *topic == name).map(|(_, label, example)| (*label, *example))
}
fn topic(name: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|topic| topic.name == name)
}
/// `info`, a category, or one topic by name; anything else is not a sheet.
pub fn resolve(query: &str) -> Option<Sheet> {
    if query == "info" {
        let info = topic("info")?;
        let mut entries = vec![("Stack formatting", "bold::blue::Hello;")];
        for name in ["timer", "remind", "checklist"] {
            let value = topic(name)?;
            entries.push((lead(value), value.example));
        }
        return Some(Sheet { title: "Getting started".into(), intro: Some(info.description.into()), entries, more: 0 });
    }
    if let Some((name, title, intro)) = CATEGORIES.iter().find(|(name, ..)| *name == query) {
        let topics = TOPICS.iter().filter(|topic| topic.category == *name).collect::<Vec<_>>();
        let mut entries = topics.iter().take(SHEET_ROWS).map(|topic| (lead(topic), topic.example)).collect::<Vec<_>>();
        let room = SHEET_ROWS.saturating_sub(entries.len());
        entries.extend(topics.iter().flat_map(|topic| examples(topic.name)).take(room));
        return Some(Sheet { title: (*title).into(), intro: Some((*intro).into()), more: topics.len().saturating_sub(SHEET_ROWS), entries });
    }
    let value = topic(query).filter(|value| value.name != "info")?;
    let options = (!value.options.is_empty()).then(|| format!("\nOptions: {}.", value.options.join(", "))).unwrap_or_default();
    let intro = format!("{}. Write {}{options}", value.description, value.template);
    let entries = std::iter::once((lead(value), value.example)).chain(examples(value.name)).collect();
    Some(Sheet { title: label(value.name).into(), intro: Some(intro), entries, more: 0 })
}
/// The message body every client derives alike, so a sheet's wording can change without breaking older messages.
pub fn body(topic: &str) -> String {
    format!("SigilText help: {topic}")
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
        for (name, _, example) in EXAMPLES.iter().filter(|(name, ..)| topic(name).is_some_and(|t| t.structured)) {
            let draft = crate::parse_card(example, origin, Default::default()).unwrap();
            assert!(matches!(draft.content, crate::Parsed::Card(_)), "{name}: {example}");
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
            let sheet = resolve(category).unwrap();
            assert!(sheet.entries.len() >= 2, "{category}");
            assert!(sheet.entries.iter().all(|(label, _)| !label.contains("::")), "{category}");
        }
    }
    #[test]
    fn help_sources_become_cards_rendered_from_the_readers_reference() {
        let origin = crate::Origin { message: [1; 32], creator: [2; 32], created_at: 1800000000, timezone: Some("UTC") };
        let draft = crate::parse_card("help::time;", origin, Default::default()).unwrap();
        let crate::Parsed::Card(card) = draft.content else { panic!("help::time; stayed text") };
        let crate::structured::Construct::Utility(utility) = &card.content else { panic!() };
        let view = utility.presentation().unwrap();
        assert_eq!(view["kind"], "help");
        assert_eq!(view["display"], "Time");
        assert_eq!(view["entries"].as_array().unwrap().len(), SHEET_ROWS);
        assert_eq!((view["entries"][1]["label"].as_str(), view["entries"][1]["code"].as_str()), (Some("Timer"), Some("timer::5m;")));
        assert_eq!(card.body().unwrap(), "SigilText help: time");
        let text = resolve("text").unwrap();
        assert_eq!((text.entries.len(), text.more), (SHEET_ROWS, TOPICS.iter().filter(|t| t.category == "text").count() - SHEET_ROWS));
        let timer = resolve("timer").unwrap();
        assert_eq!((timer.title.as_str(), timer.entries), ("Timer", vec![("Timer", "timer::5m;"), ("Longer timer", "timer::1h30m;")]));
        assert!(timer.intro.unwrap().contains("timer::<duration>;"));
        let diagrams = resolve("diagrams").unwrap();
        assert_eq!(diagrams.entries.iter().map(|e| e.0).collect::<Vec<_>>(), ["Flowchart", "Sequence", "Timeline", "Mind map", "Org chart", "State machine"]);
        assert_eq!(resolve("data").unwrap().entries.iter().map(|e| e.0).collect::<Vec<_>>(), ["Pie chart", "Table", "Bar chart", "Line chart"]);
        for (name, ..) in EXAMPLES {
            assert!(topic(name).is_some() && label(name) != "Example", "{name}");
        }
        for value in TOPICS {
            assert_ne!(label(value.name), "Example", "{}", value.name);
        }
        for literal in ["help::nope;", "help::Time;", "help::redact::SECRET;"] {
            assert!(matches!(crate::parse_card(literal, origin, Default::default()).unwrap().content, crate::Parsed::Text(_)), "{literal}");
        }
        assert!(crate::utility::Utility::Help("time;".into()).validate(Default::default()).is_err());
        let unknown = crate::utility::Utility::Help("later".into()).presentation().unwrap();
        assert_eq!(unknown["entries"].as_array().unwrap().len(), 0);
    }
}
