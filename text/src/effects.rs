use crate::Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hue {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Pink,
    Gray,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Color {
    pub hue: Hue,
    pub shade: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Paint {
    Solid { color: Color },
    Gradient { stops: Vec<Color> },
    Rainbow,
    Theme,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Animation {
    Shake,
    Wave,
    Pulse,
    Glow,
    Typewriter,
    Sparkle,
    Glitch,
    Scatter,
    Flip,
    Barrel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reveal {
    Spoiler,
    Scratch,
}
/// Redaction has no retained secret or reveal state in this model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "Vec<Descriptor>", try_from = "Vec<Descriptor>")]
pub struct Effects {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: bool,
    pub mono: bool,
    pub code: bool,
    pub mark: bool,
    pub paint: Option<Paint>,
    pub size: Option<i8>,
    pub animation: Option<Animation>,
    pub reveal: Option<Reveal>,
    pub link: Option<String>,
}
impl Color {
    fn parse(s: &str) -> Option<Self> {
        let (name, shade) = match s.as_bytes().last()? {
            b'1'..=b'3' => (&s[..s.len() - 1], s.as_bytes()[s.len() - 1] - b'0'),
            _ => (s, 2),
        };
        let hue = match name {
            "red" => Hue::Red,
            "orange" => Hue::Orange,
            "yellow" => Hue::Yellow,
            "green" => Hue::Green,
            "cyan" => Hue::Cyan,
            "blue" => Hue::Blue,
            "purple" => Hue::Purple,
            "pink" => Hue::Pink,
            "gray" => Hue::Gray,
            _ => return None,
        };
        Some(Self { hue, shade })
    }
}
impl Effects {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let valid_color = |c: &Color| (1..=3).contains(&c.shade);
        if self.size.is_some_and(|s| s == 0 || !(-3..=3).contains(&s))
            || self.link.as_ref().is_some_and(|s| !valid_link(s))
        {
            return Err(Error::Invalid);
        }
        match &self.paint {
            Some(Paint::Theme) => return Err(Error::Invalid),
            Some(Paint::Solid { color }) if !valid_color(color) => return Err(Error::Invalid),
            Some(Paint::Gradient { stops })
                if !(2..=9).contains(&stops.len()) || !stops.iter().all(valid_color) =>
            {
                return Err(Error::Invalid)
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn overlay(&mut self, other: &Self) {
        self.bold |= other.bold;
        self.italic |= other.italic;
        self.strike |= other.strike;
        self.underline |= other.underline;
        self.mono |= other.mono;
        self.code |= other.code;
        self.mark |= other.mark;
        if other.paint.is_some() {
            self.paint.clone_from(&other.paint)
        }
        if other.size.is_some() {
            self.size = other.size
        }
        if other.animation.is_some() {
            self.animation = other.animation
        }
        if other.reveal.is_some() {
            self.reveal = other.reveal
        }
        if other.link.is_some() {
            self.link.clone_from(&other.link)
        }
    }
    pub(crate) fn modifier(&mut self, s: &str) -> bool {
        match s {
            "bold" => self.bold = true,
            "italic" => self.italic = true,
            "strike" => self.strike = true,
            "underline" => self.underline = true,
            "mono" => self.mono = true,
            "mark" => self.mark = true,
            "spoiler" => self.reveal = Some(Reveal::Spoiler),
            "scratch" => self.reveal = Some(Reveal::Scratch),
            "small" | "small2" => self.size = Some(-2),
            "small1" => self.size = Some(-1),
            "small3" => self.size = Some(-3),
            "big" | "big2" => self.size = Some(2),
            "big1" => self.size = Some(1),
            "big3" => self.size = Some(3),
            "shake" => self.animation = Some(Animation::Shake),
            "wave" => self.animation = Some(Animation::Wave),
            "pulse" => self.animation = Some(Animation::Pulse),
            "glow" => self.animation = Some(Animation::Glow),
            "typewriter" => self.animation = Some(Animation::Typewriter),
            "sparkle" => self.animation = Some(Animation::Sparkle),
            "glitch" => self.animation = Some(Animation::Glitch),
            "scatter" => self.animation = Some(Animation::Scatter),
            "flip" => self.animation = Some(Animation::Flip),
            "barrel" => self.animation = Some(Animation::Barrel),
            "rainbow" => self.paint = Some(Paint::Rainbow),
            _ => {
                if let Some(color) = Color::parse(s) {
                    self.paint = Some(Paint::Solid { color });
                } else if s.contains('-') && s.len() <= 80 {
                    let Some(stops) = s.split('-').map(Color::parse).collect::<Option<Vec<_>>>()
                    else {
                        return false;
                    };
                    if !(2..=9).contains(&stops.len()) {
                        return false;
                    }
                    self.paint = Some(Paint::Gradient { stops });
                } else {
                    return false;
                }
            }
        }
        true
    }
}
// Only explicit web/mail links are actionable. Relative URLs and raw HTML never are.
pub(crate) fn valid_link(s: &str) -> bool {
    if s.len() > 2048
        || !s
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\'' | b'<' | b'>' | b'\\'))
    {
        return false;
    }
    if let Some(rest) = s.strip_prefix("mailto:") {
        return !rest.is_empty() && !rest.starts_with('?');
    }
    let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !authority.contains('@')
}

impl From<Color> for String {
    fn from(color: Color) -> Self {
        let name = match color.hue {
            Hue::Red => "red",
            Hue::Orange => "orange",
            Hue::Yellow => "yellow",
            Hue::Green => "green",
            Hue::Cyan => "cyan",
            Hue::Blue => "blue",
            Hue::Purple => "purple",
            Hue::Pink => "pink",
            Hue::Gray => "gray",
        };
        format!("{name}{}", color.shade)
    }
}
impl TryFrom<String> for Color {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let color = Color::parse(&value).ok_or("invalid color")?;
        if String::from(color) != value {
            return Err("noncanonical color");
        }
        Ok(color)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Emphasis {
    Bold,
    Italic,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Decoration {
    Strike,
    Underline,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Descriptor {
    Color(Paint),
    Background(Paint),
    Emphasis(Emphasis),
    Decoration(Decoration),
    Monospace(bool),
    Code(bool),
    Size(i8),
    Animation(Animation),
    Reveal(Reveal),
    Link(String),
}
impl From<Effects> for Vec<Descriptor> {
    fn from(e: Effects) -> Self {
        let mut result = Vec::new();
        if e.mark {
            result.push(Descriptor::Background(e.paint.unwrap_or(Paint::Theme)))
        } else if let Some(paint) = e.paint {
            result.push(Descriptor::Color(paint))
        }
        for (enabled, descriptor) in [
            (e.bold, Descriptor::Emphasis(Emphasis::Bold)),
            (e.italic, Descriptor::Emphasis(Emphasis::Italic)),
            (e.strike, Descriptor::Decoration(Decoration::Strike)),
            (e.underline, Descriptor::Decoration(Decoration::Underline)),
            (e.mono, Descriptor::Monospace(true)),
            (e.code, Descriptor::Code(true)),
        ] {
            if enabled {
                result.push(descriptor)
            }
        }
        if let Some(size) = e.size {
            result.push(Descriptor::Size(size))
        }
        if let Some(animation) = e.animation {
            result.push(Descriptor::Animation(animation))
        }
        if let Some(reveal) = e.reveal {
            result.push(Descriptor::Reveal(reveal))
        }
        if let Some(link) = e.link {
            result.push(Descriptor::Link(link))
        }
        result
    }
}
impl TryFrom<Vec<Descriptor>> for Effects {
    type Error = &'static str;
    fn try_from(descriptors: Vec<Descriptor>) -> Result<Self, Self::Error> {
        if descriptors.len() > 11 {
            return Err("too many effects");
        }
        let mut e = Effects::default();
        let mut paint = false;
        macro_rules! flag {
            ($field:ident) => {
                if e.$field {
                    return Err("duplicate effect");
                } else {
                    e.$field = true
                }
            };
        }
        macro_rules! value {
            ($field:ident,$value:expr) => {
                if e.$field.replace($value).is_some() {
                    return Err("duplicate effect");
                }
            };
        }
        for descriptor in descriptors {
            match descriptor {
                Descriptor::Color(p) => {
                    if paint {
                        return Err("duplicate paint");
                    }
                    paint = true;
                    e.paint = Some(p);
                }
                Descriptor::Background(p) => {
                    if paint {
                        return Err("duplicate paint");
                    }
                    paint = true;
                    e.mark = true;
                    e.paint = if p == Paint::Theme { None } else { Some(p) };
                }
                Descriptor::Emphasis(Emphasis::Bold) => flag!(bold),
                Descriptor::Emphasis(Emphasis::Italic) => flag!(italic),
                Descriptor::Decoration(Decoration::Strike) => flag!(strike),
                Descriptor::Decoration(Decoration::Underline) => flag!(underline),
                Descriptor::Monospace(true) => flag!(mono),
                Descriptor::Code(true) => flag!(code),
                Descriptor::Monospace(false) | Descriptor::Code(false) => {
                    return Err("inactive effect")
                }
                Descriptor::Size(v) => value!(size, v),
                Descriptor::Animation(v) => value!(animation, v),
                Descriptor::Reveal(v) => value!(reveal, v),
                Descriptor::Link(v) => value!(link, v),
            }
        }
        e.validate().map_err(|_| "invalid effects")?;
        Ok(e)
    }
}
