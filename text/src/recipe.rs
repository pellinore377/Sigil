use crate::{Error, Text, data::Recipe, numeric::Number};
use serde_json::{Value, json};

fn quantity(source: &str) -> Option<(usize, f64)> {
    let token = source
        .bytes()
        .take_while(|b| b.is_ascii_digit() || matches!(b, b'.' | b'/'))
        .count();
    if token == 0 {
        return None;
    }
    let fraction = |value: &str| -> Option<f64> {
        if let Some((a, b)) = value.split_once('/') {
            let a: u32 = a.parse().ok()?;
            let b: u32 = b.parse().ok()?;
            (b > 0).then_some(f64::from(a) / f64::from(b))
        } else {
            value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
        }
    };
    let mut end = token;
    let mut value = fraction(&source[..token])?;
    if !source[..token].contains(['.', '/']) && source.as_bytes().get(token) == Some(&b' ') {
        let second = source[token + 1..]
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'/')
            .count();
        let part = &source[token + 1..token + 1 + second];
        if part.contains('/') {
            value += fraction(part)?;
            end = token + 1 + second;
        }
    }
    if source[end..]
        .chars()
        .next()
        .is_some_and(|c| !c.is_whitespace() && !c.is_alphabetic())
    {
        return None;
    }
    Some((end, value))
}
impl Recipe {
    pub fn presentation(&self, serves: Option<u16>) -> Result<Value, Error> {
        let target = serves.or(self.serves);
        if serves == Some(0) || serves.is_some() && self.serves.is_none() {
            return Err(Error::Invalid);
        }
        let mut ingredients = self.ingredients.clone();
        let mut scaled = vec![false; ingredients.len()];
        if let (Some(original), Some(target)) = (self.serves, target) {
            if original != target {
                for (index, text) in ingredients.iter_mut().enumerate() {
                    let Some((end, value)) = quantity(text.body()) else {
                        continue;
                    };
                    let Ok(number) = Number::new(value * f64::from(target) / f64::from(original))
                    else {
                        continue;
                    };
                    let replacement = number.fixed(3)?;
                    if value > 0.0 && replacement == "0" {
                        continue;
                    }
                    if let Some(value) = text.replace_plain_prefix(end, &replacement)? {
                        *text = value;
                        scaled[index] = true;
                    }
                }
            }
        }
        Ok(
            json!({"title":self.title.presentation(), "serves":target, "original_serves":self.serves, "seconds":self.seconds,
            "ingredients":ingredients.iter().map(Text::presentation).collect::<Vec<_>>(), "scaled":scaled,
            "steps":self.steps.iter().map(Text::presentation).collect::<Vec<_>>()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serving_changes_scale_only_clear_visible_quantities_without_mutating_the_recipe() {
        let plain = |s| Text::plain(s, Default::default()).unwrap();
        let value = Recipe {
            title: plain("Dinner"),
            serves: Some(4),
            seconds: Some(1200),
            ingredients: vec![
                plain("200g flour"),
                plain("1 1/2 cups milk"),
                plain("1-2 eggs"),
                plain("Salt to taste"),
                crate::parse("spoiler::2; secret ingredients", Default::default()).unwrap(),
                crate::parse("2 **fresh** lemons", Default::default()).unwrap(),
            ],
            steps: vec![plain("Combine.")],
        };
        let original = serde_json::to_vec(&value).unwrap();
        let view = value.presentation(Some(2)).unwrap();
        assert_eq!(view["ingredients"][0]["text"], "100g flour");
        assert_eq!(view["ingredients"][1]["text"], "0.75 cups milk");
        assert_eq!(view["ingredients"][2]["text"], "1-2 eggs");
        assert_eq!(view["ingredients"][3]["text"], "Salt to taste");
        assert_eq!(
            view["scaled"],
            json!([true, true, false, false, false, true])
        );
        assert_eq!(view["ingredients"][5]["spans"][0]["start"], 2);
        assert_eq!(
            view["ingredients"][4]["spans"][0]["effects"][0]["kind"],
            "reveal"
        );
        assert_eq!(serde_json::to_vec(&value).unwrap(), original);
        assert!(value.presentation(Some(0)).is_err());
        assert!(value.presentation(Some(u16::MAX)).is_ok());
        assert_eq!(
            value.presentation(Some(4)).unwrap()["ingredients"][1]["text"],
            "1 1/2 cups milk"
        );
        for source in ["1/0 cup", "1..2 cups", "1–2 eggs", "-2 cups", ". cups"] {
            assert!(quantity(source).is_none());
        }
    }
}
