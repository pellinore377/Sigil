use crate::{
    numeric::{calculate, Conversion, Number},
    structured::CardLimits,
    Error, Text,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Utility {
    Calculation {
        expression: Text,
        result: Number,
    },
    Conversion(Conversion),
    Math {
        expression: String,
        block: bool,
    },
    Art(String),
    Qr(Qr),
    Random(Randomizer),
    Swatch([u8; 4]),
    Keys(Vec<Text>),
    Rating {
        value: Number,
        max: Number,
    },
    Progress(Number),
    Quote {
        author: Text,
        source: Option<Text>,
        text: Text,
    },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Qr {
    Text {
        text: Text,
    },
    Url {
        url: String,
    },
    Wifi {
        ssid: Text,
        password: Text,
    },
    Contact {
        address: String,
        #[serde(with = "crate::structured::id")]
        identity: [u8; 32],
    },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Randomizer {
    Dice {
        groups: Vec<Dice>,
    },
    Pick {
        category: Option<String>,
        options: Vec<Text>,
        selected: u16,
    },
    Number {
        min: i64,
        max: i64,
        selected: i64,
    },
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dice {
    pub sides: u32,
    pub faces: Vec<u32>,
}
fn random(max: u64) -> Result<u64, Error> {
    if max == 0 {
        return Err(Error::Invalid);
    }
    let threshold = max.wrapping_neg() % max;
    for _ in 0..128 {
        let mut bytes = [0; 8];
        getrandom::fill(&mut bytes).map_err(|_| Error::Invalid)?;
        let value = u64::from_le_bytes(bytes);
        if value >= threshold {
            return Ok(value % max);
        }
    }
    Err(Error::Invalid)
}
impl Randomizer {
    pub fn dice(source: &str, limits: CardLimits) -> Result<Self, Error> {
        limits.validate()?;
        if source.len() > 1024 {
            return Err(Error::Limit);
        }
        let mut groups = Vec::new();
        let mut total = 0usize;
        for group in source.split(',') {
            let (count, sides) = group.trim().split_once('d').ok_or(Error::Invalid)?;
            let count = count.parse::<usize>().map_err(|_| Error::Invalid)?;
            let sides = sides.parse::<u32>().map_err(|_| Error::Invalid)?;
            total = total.checked_add(count).ok_or(Error::Limit)?;
            if count == 0 || total > limits.items || !(2..=limits.dice_sides).contains(&sides) {
                return Err(Error::Limit);
            }
            let faces = (0..count)
                .map(|_| random(u64::from(sides)).map(|v| v as u32 + 1))
                .collect::<Result<_, _>>()?;
            groups.push(Dice { sides, faces });
        }
        Ok(Self::Dice { groups })
    }
    pub fn pick(
        category: Option<String>,
        options: Vec<Text>,
        limits: CardLimits,
    ) -> Result<Self, Error> {
        limits.validate()?;
        if options.len() > limits.items {
            return Err(Error::Limit);
        }
        let mut unique = Vec::new();
        let mut seen = BTreeSet::new();
        for option in options {
            if !option.body().trim().is_empty() && seen.insert(option.body().to_owned()) {
                unique.push(option);
            }
        }
        if unique.is_empty() || unique.len() > limits.items {
            return Err(Error::Limit);
        }
        let selected = random(unique.len() as u64)? as u16;
        let value = Self::Pick {
            category,
            options: unique,
            selected,
        };
        value.validate(limits)?;
        Ok(value)
    }
    pub fn number(min: i64, max: i64) -> Result<Self, Error> {
        let count = i128::from(max) - i128::from(min) + 1;
        if !(1..=u64::MAX as i128).contains(&count) {
            return Err(Error::Invalid);
        }
        let selected = (i128::from(min) + i128::from(random(count as u64)?)) as i64;
        Ok(Self::Number { min, max, selected })
    }
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        match self {
            Self::Dice { groups } => {
                if groups.is_empty() || groups.len() > limits.items {
                    return Err(Error::Limit);
                }
                let mut total = 0usize;
                for group in groups {
                    total = total.checked_add(group.faces.len()).ok_or(Error::Limit)?;
                    if group.faces.is_empty()
                        || total > limits.items
                        || !(2..=limits.dice_sides).contains(&group.sides)
                        || group.faces.iter().any(|&v| v == 0 || v > group.sides)
                    {
                        return Err(Error::Invalid);
                    }
                }
            }
            Self::Pick {
                category,
                options,
                selected,
            } => {
                if category.as_ref().is_some_and(|s| {
                    s.is_empty()
                        || s.len() > 64
                        || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                }) || options.is_empty()
                    || options.len() > limits.items
                    || *selected as usize >= options.len()
                {
                    return Err(Error::Invalid);
                }
                let mut unique = BTreeSet::new();
                for option in options {
                    if option.body().trim().is_empty() || !unique.insert(option.body()) {
                        return Err(Error::Invalid);
                    }
                }
            }
            Self::Number { min, max, selected } => {
                if min > max || selected < min || selected > max {
                    return Err(Error::Invalid);
                }
            }
        }
        Ok(())
    }
    fn body(&self) -> String {
        match self {
            Self::Dice { groups } => {
                let mut sum = 0u64;
                let rows = groups
                    .iter()
                    .map(|g| {
                        let subtotal = g.faces.iter().map(|&n| u64::from(n)).sum::<u64>();
                        sum += subtotal;
                        format!(
                            "{}d{}: {} = {subtotal}",
                            g.faces.len(),
                            g.sides,
                            g.faces
                                .iter()
                                .map(u32::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .collect::<Vec<_>>();
                format!("{}\nTotal: {sum}", rows.join("\n"))
            }
            Self::Pick {
                category,
                options,
                selected,
            } => format!(
                "{}: {}",
                category.as_deref().unwrap_or("Pick"),
                options[*selected as usize].body()
            ),
            Self::Number { min, max, selected } => format!("Pick ({min}–{max}): {selected}"),
        }
    }
}
impl Qr {
    pub fn payload(&self) -> Result<String, Error> {
        let escape = |s: &str| {
            s.chars()
                .flat_map(|c| {
                    if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                        vec!['\\', c]
                    } else {
                        vec![c]
                    }
                })
                .collect::<String>()
        };
        let payload = match self {
            Self::Text { text } => text.body().into(),
            Self::Url { url } => {
                if !crate::effects::valid_link(url) || !url.starts_with("https://") {
                    return Err(Error::Invalid);
                }
                url.clone()
            }
            Self::Wifi { ssid, password } => {
                if ssid.body().is_empty() || ssid.body().len() > 32 || password.body().len() > 63 {
                    return Err(Error::Invalid);
                }
                format!(
                    "WIFI:T:{};S:{};P:{};;",
                    if password.body().is_empty() {
                        "nopass"
                    } else {
                        "WPA"
                    },
                    escape(ssid.body()),
                    escape(password.body())
                )
            }
            Self::Contact { address, identity } => {
                if *identity == [0; 32] || !crate::contact::valid_address(address) {
                    return Err(Error::Invalid);
                }
                format!(
                    "sigil:contact:{}:{}",
                    address,
                    identity
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                )
            }
        };
        if payload.is_empty() || payload.len() > 2048 {
            return Err(Error::Limit);
        }
        Ok(payload)
    }
    /// Includes the four-module light quiet zone. Never invert for dark mode.
    pub fn modules(&self) -> Result<(usize, Vec<bool>), Error> {
        let code = qrcode::QrCode::with_error_correction_level(
            self.payload()?.as_bytes(),
            qrcode::EcLevel::M,
        )
        .map_err(|_| Error::Limit)?;
        let width = code.width() + 8;
        let mut cells = vec![false; width * width];
        for (i, value) in code.to_colors().into_iter().enumerate() {
            cells[(i / code.width() + 4) * width + i % code.width() + 4] =
                value == qrcode::Color::Dark;
        }
        Ok((width, cells))
    }
}
impl Utility {
    pub fn html(&self) -> Result<String, Error> {
        self.validate(Default::default())?;
        if let Self::Math { expression, block } = self {
            crate::math::html(expression, *block)
        } else if let Self::Art(text) = self {
            let mut out = String::from("<pre>");
            crate::model::attribute(&mut out, text);
            out.push_str("</pre>");
            Ok(out)
        } else {
            Ok(Text::plain(&self.body()?, Default::default())?.html())
        }
    }
    pub fn texts(&self) -> Vec<&Text> {
        match self {
            Self::Calculation { expression, .. } => vec![expression],
            Self::Qr(Qr::Text { text }) => vec![text],
            Self::Qr(Qr::Wifi { ssid, password }) => vec![ssid, password],
            Self::Random(Randomizer::Pick { options, .. }) | Self::Keys(options) => {
                options.iter().collect()
            }
            Self::Quote {
                author,
                source,
                text,
            } => std::iter::once(author)
                .chain(source)
                .chain(std::iter::once(text))
                .collect(),
            _ => Vec::new(),
        }
    }
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        match self {
            Self::Calculation { expression, result } => {
                if calculate(expression.body())? != *result {
                    return Err(Error::Invalid);
                }
            }
            Self::Conversion(value) => value.validate()?,
            Self::Math { expression, .. } => {
                if Text::plain(expression, limits.text)?.body() != expression {
                    return Err(Error::Invalid);
                }
                crate::math::html(expression, false)?;
            }
            Self::Art(value) => {
                if value.is_empty()
                    || Text::plain(value, limits.text)?.body() != value
                    || value
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    return Err(Error::Invalid);
                }
            }
            Self::Qr(value) => {
                value.payload()?;
            }
            Self::Random(value) => value.validate(limits)?,
            Self::Keys(keys) => {
                if keys.is_empty()
                    || keys.len() > 16
                    || keys
                        .iter()
                        .any(|v| v.body().is_empty() || v.body().len() > 64)
                {
                    return Err(Error::Invalid);
                }
            }
            Self::Rating { value, max } => {
                if value.value() < 0.0 || max.value() <= 0.0 || value.value() > max.value() {
                    return Err(Error::Invalid);
                }
            }
            Self::Progress(value) => {
                if !(0.0..=100.0).contains(&value.value()) {
                    return Err(Error::Invalid);
                }
            }
            Self::Quote { author, text, .. } => {
                if author.body().trim().is_empty() || text.body().trim().is_empty() {
                    return Err(Error::Invalid);
                }
            }
            Self::Swatch(_) => (),
        }
        Ok(())
    }
    pub(crate) fn body(&self) -> Result<String, Error> {
        Ok(match self {
            Self::Calculation { expression, result } => {
                format!("{} = {}", expression.body(), result.fixed(6)?)
            }
            Self::Conversion(value) => value.body()?,
            Self::Math { expression, .. } | Self::Art(expression) => expression.clone(),
            Self::Qr(value) => format!("QR: {}", value.payload()?),
            Self::Random(value) => value.body(),
            Self::Swatch(rgba) => format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                rgba[0], rgba[1], rgba[2], rgba[3]
            ),
            Self::Keys(keys) => keys.iter().map(Text::body).collect::<Vec<_>>().join("+"),
            Self::Rating { value, max } => format!("{}/{}", value.as_str(), max.as_str()),
            Self::Progress(value) => format!("{}%", value.as_str()),
            Self::Quote {
                author,
                source,
                text,
            } => format!(
                "{}\n— {}{}",
                text.body(),
                author.body(),
                source
                    .as_ref()
                    .map(|v| format!(", {}", v.body()))
                    .unwrap_or_default()
            ),
        })
    }
}

pub fn category(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "food" => &[
            "pizza",
            "tacos",
            "burgers",
            "sushi",
            "pasta",
            "curry",
            "sandwiches",
            "barbecue",
            "salad",
            "noodles",
            "breakfast",
            "seafood",
        ],
        "movie" => &[
            "action",
            "comedy",
            "drama",
            "thriller",
            "horror",
            "sci-fi",
            "fantasy",
            "animation",
            "documentary",
            "mystery",
            "romance",
            "adventure",
        ],
        "book" => &[
            "fiction",
            "mystery",
            "sci-fi",
            "fantasy",
            "history",
            "biography",
            "science",
            "philosophy",
            "horror",
            "romance",
            "thriller",
            "graphic novel",
        ],
        "activity" => &[
            "walk",
            "movie",
            "game",
            "cook",
            "read",
            "exercise",
            "café",
            "museum",
            "drive",
            "picnic",
            "photography",
            "music",
        ],
        "chore" => &[
            "dishes",
            "laundry",
            "vacuum",
            "trash",
            "bathroom",
            "kitchen",
            "dusting",
            "organizing",
            "groceries",
            "yard work",
        ],
        "meal" => &["breakfast", "brunch", "lunch", "dinner", "snack", "dessert"],
        "color" => &[
            "red", "orange", "yellow", "green", "cyan", "blue", "purple", "pink", "gray",
        ],
        "direction" => &["north", "south", "east", "west"],
        "yesno" => &["yes", "no"],
        "flip" => &["Heads", "Tails"],
        _ => return None,
    })
}
pub(crate) fn parse(lines: &[&str], limits: CardLimits) -> Result<Utility, Error> {
    let (kind, source) = lines[0].split_once("::").ok_or(Error::Invalid)?;
    let text = |s: &str| crate::parse(s, limits.text);
    if lines.len() != 1 && kind != "art" && !(kind == "math" && source == "block") {
        return Err(Error::Invalid);
    }
    Ok(match kind {
        "calc" => {
            let expression = text(source)?;
            let result = calculate(expression.body())?;
            Utility::Calculation { expression, result }
        }
        "convert" => Utility::Conversion(Conversion::parse(text(source)?.body())?),
        "math" => {
            let block = source == "block";
            let raw = if block {
                lines[1..].join("\n")
            } else {
                source.into()
            };
            Utility::Math {
                expression: Text::plain(&raw, limits.text)?.body().into(),
                block,
            }
        }
        "art" => {
            if !source.is_empty() || lines.last() != Some(&"") {
                return Err(Error::Invalid);
            }
            Utility::Art(
                Text::plain(&lines[1..lines.len() - 1].join("\n"), limits.text)?
                    .body()
                    .into(),
            )
        }
        "roll" => Utility::Random(Randomizer::dice(source, limits)?),
        "pick" => {
            let value = if let Some(range) = source.strip_prefix("number::") {
                let at = range
                    .get(1..)
                    .ok_or(Error::Invalid)?
                    .find('-')
                    .ok_or(Error::Invalid)?
                    + 1;
                Randomizer::number(
                    range[..at].parse().map_err(|_| Error::Invalid)?,
                    range[at + 1..].parse().map_err(|_| Error::Invalid)?,
                )?
            } else if let Some(options) = category(source) {
                Randomizer::pick(
                    Some(source.into()),
                    options.iter().map(|v| text(v)).collect::<Result<_, _>>()?,
                    limits,
                )?
            } else {
                Randomizer::pick(
                    None,
                    source
                        .split(',')
                        .map(|v| text(v.trim()))
                        .collect::<Result<_, _>>()?,
                    limits,
                )?
            };
            Utility::Random(value)
        }
        "qr" => Utility::Qr(if let Some(rest) = source.strip_prefix("wifi::") {
            let (ssid, password) = rest.split_once("::").ok_or(Error::Invalid)?;
            Qr::Wifi {
                ssid: text(ssid)?,
                password: text(password)?,
            }
        } else if let Some(rest) = source.strip_prefix("text::") {
            Qr::Text { text: text(rest)? }
        } else {
            Qr::Url {
                url: text(source)?.body().into(),
            }
        }),
        "kbd" => Utility::Keys(
            source
                .split('+')
                .map(|v| text(v.trim()))
                .collect::<Result<_, _>>()?,
        ),
        "rate" => {
            let (value, max) = source.split_once('/').ok_or(Error::Invalid)?;
            Utility::Rating {
                value: Number::try_from(value.to_owned())?,
                max: Number::try_from(max.to_owned())?,
            }
        }
        "progress" => Utility::Progress(Number::new(
            Number::try_from(source.to_owned())?
                .value()
                .clamp(0.0, 100.0),
        )?),
        "quote" => {
            let parts = source.splitn(3, "::").collect::<Vec<_>>();
            match parts.as_slice() {
                [author, value] => Utility::Quote {
                    author: text(author)?,
                    source: None,
                    text: text(value)?,
                },
                [author, source, value] => Utility::Quote {
                    author: text(author)?,
                    source: Some(text(source)?),
                    text: text(value)?,
                },
                _ => return Err(Error::Invalid),
            }
        }
        "swatch" => Utility::Swatch(swatch(source)?),
        _ => return Err(Error::Invalid),
    })
}
fn swatch(source: &str) -> Result<[u8; 4], Error> {
    if let Some(hex) = source.strip_prefix('#') {
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let mut rgba = [0, 0, 0, 255];
        for (i, chunk) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            rgba[i] =
                u8::from_str_radix(std::str::from_utf8(chunk).map_err(|_| Error::Invalid)?, 16)
                    .map_err(|_| Error::Invalid)?;
        }
        return Ok(rgba);
    }
    let (kind, rest) = source.split_once('(').ok_or(Error::Invalid)?;
    let parts = rest
        .strip_suffix(')')
        .ok_or(Error::Invalid)?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if !matches!((kind, parts.len()), ("rgb", 3) | ("rgba", 4) | ("hsl", 3)) {
        return Err(Error::Invalid);
    }
    if kind == "hsl" {
        let h = Number::try_from(parts[0].to_owned())?
            .value()
            .rem_euclid(360.0)
            / 60.0;
        let s = Number::try_from(parts[1].strip_suffix('%').ok_or(Error::Invalid)?.to_owned())?
            .value()
            / 100.0;
        let l = Number::try_from(parts[2].strip_suffix('%').ok_or(Error::Invalid)?.to_owned())?
            .value()
            / 100.0;
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&l) {
            return Err(Error::Invalid);
        }
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let m = l - c / 2.0;
        let rgb = match h as u8 {
            0 => [c, x, 0.0],
            1 => [x, c, 0.0],
            2 => [0.0, c, x],
            3 => [0.0, x, c],
            4 => [x, 0.0, c],
            _ => [c, 0.0, x],
        };
        return Ok([
            ((rgb[0] + m) * 255.0).round() as u8,
            ((rgb[1] + m) * 255.0).round() as u8,
            ((rgb[2] + m) * 255.0).round() as u8,
            255,
        ]);
    }
    let mut rgba = [0, 0, 0, 255];
    for i in 0..3 {
        rgba[i] = parts[i].parse().map_err(|_| Error::Invalid)?;
    }
    if parts.len() == 4 {
        let alpha = Number::try_from(parts[3].to_owned())?.value();
        if !(0.0..=1.0).contains(&alpha) {
            return Err(Error::Invalid);
        }
        rgba[3] = (alpha * 255.0).round() as u8;
    }
    Ok(rgba)
}
