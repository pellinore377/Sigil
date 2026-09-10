use crate::{
    Error, Text,
    utility::{Qr, Randomizer, Utility},
};
use serde_json::{Value, json};

impl Utility {
    pub fn presentation(&self) -> Result<Value, Error> {
        self.validate(Default::default())?;
        let plain =
            |text: &str| Text::plain(text, Default::default()).map(|v| json!(v.presentation()));
        Ok(match self {
            Self::Calculation { expression, result } => {
                json!({"kind":"calculation","rich":expression.presentation(),"display":result.fixed(6)?,"copy":result.as_str()})
            }
            Self::Conversion(value) => {
                json!({"kind":"conversion","display":format!("{} {}",value.input.as_str(),value.from),"alternate":format!("{} {}",value.output.fixed(4)?,value.to),"copy":format!("{} {}",value.output.as_str(),value.to)})
            }
            Self::Math { expression, block } => {
                json!({"kind":"math","display":expression,"copy":expression,"mathml":crate::math::html(expression,*block)?})
            }
            Self::Art(value) => json!({"kind":"art","display":value,"copy":value}),
            Self::Qr(value) => {
                let (width, modules) = value.modules()?;
                let (kind, label, password, concealed) = match value {
                    Qr::Text { text } => (
                        "text",
                        plain("Text QR code")?,
                        Value::Null,
                        text.spans().iter().any(|s| s.effects.reveal.is_some()),
                    ),
                    Qr::Url { url } => ("url", plain(url)?, Value::Null, false),
                    Qr::Wifi { ssid, password } => (
                        "wifi",
                        json!(ssid.presentation()),
                        json!(password.presentation()),
                        ssid.spans()
                            .iter()
                            .chain(password.spans())
                            .any(|s| s.effects.reveal.is_some()),
                    ),
                    Qr::Contact { address, .. } => ("contact", plain(address)?, Value::Null, false),
                };
                json!({"kind":"qr","rich":label,"qr":{"kind":kind,"width":width,"cells":modules.iter().map(|&v|if v{'1'}else{'0'}).collect::<String>(),"payload":value.payload()?,"password":password,"concealed":concealed}})
            }
            Self::Random(value) => match value {
                Randomizer::Dice { groups } => {
                    json!({"kind":"dice","display":self.body()?,"details":groups.iter().flat_map(|g|g.faces.iter().map(|face|plain(&format!("d{} · {}",g.sides,face)))).collect::<Result<Vec<_>,_>>()?})
                }
                Randomizer::Pick {
                    category,
                    options,
                    selected,
                } => {
                    json!({"kind":"pick","display":category.as_deref().unwrap_or("Choice"),"rich":options[*selected as usize].presentation(),"details":options.iter().map(Text::presentation).collect::<Vec<_>>(),"selected":selected})
                }
                Randomizer::Number { min, max, selected } => {
                    json!({"kind":"random","display":selected.to_string(),"copy":selected.to_string(),"alternate":format!("Between {min} and {max}")})
                }
            },
            Self::Swatch(rgba) => {
                json!({"kind":"swatch","display":self.body()?,"copy":self.body()?,"rgba":u32::from_be_bytes(*rgba)})
            }
            Self::Keys(keys) => {
                json!({"kind":"keys","details":keys.iter().map(Text::presentation).collect::<Vec<_>>()})
            }
            Self::Rating { value, max } => {
                json!({"kind":"rating","display":self.body()?,"copy":self.body()?,"ratio":value.value()/max.value()})
            }
            Self::Progress(value) => {
                json!({"kind":"progress","display":self.body()?,"copy":self.body()?,"ratio":value.value()/100.0})
            }
            Self::Quote {
                author,
                source,
                text,
            } => {
                json!({"kind":"quote","rich":text.presentation(),"secondary":author.presentation(),"details":source.iter().map(Text::presentation).collect::<Vec<_>>()})
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utility_views_keep_committed_random_results_and_preserve_canonical_qr_fallbacks() {
        let plain = |s| Text::plain(s, Default::default()).unwrap();
        let wifi = Utility::Qr(Qr::Wifi {
            ssid: plain("Synthetic network"),
            password: plain("synthetic-secret"),
        });
        let view = wifi.presentation().unwrap();
        assert_eq!(
            wifi.body().unwrap(),
            "QR: WIFI:T:WPA;S:Synthetic network;P:synthetic-secret;;"
        );
        let card = crate::structured::Card {
            id: [1; 32],
            creator: [2; 32],
            created_at: 1_800_000_000,
            content: crate::structured::Construct::Utility(wifi.clone()),
        };
        let bytes = card.to_bytes().unwrap();
        let wire: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            wire["body"],
            "QR: WIFI:T:WPA;S:Synthetic network;P:synthetic-secret;;"
        );
        assert!(crate::structured::Card::from_bytes(&bytes).unwrap() == card);
        assert!(
            view["qr"]["payload"]
                .as_str()
                .unwrap()
                .contains("synthetic-secret")
        );
        let width = view["qr"]["width"].as_u64().unwrap() as usize;
        let cells = view["qr"]["cells"].as_str().unwrap().as_bytes();
        assert_eq!(cells.len(), width * width);
        for y in 0..width {
            for x in 0..width {
                if x < 4 || y < 4 || x >= width - 4 || y >= width - 4 {
                    assert_eq!(cells[y * width + x], b'0');
                }
            }
        }
        let hidden = Utility::Qr(Qr::Text {
            text: crate::parse("spoiler::Hidden;", Default::default()).unwrap(),
        });
        assert_eq!(hidden.presentation().unwrap()["qr"]["concealed"], true);
        let dice = Utility::Random(Randomizer::Dice {
            groups: vec![crate::utility::Dice {
                sides: 20,
                faces: vec![4, 17],
            }],
        });
        assert_eq!(dice.presentation().unwrap(), dice.presentation().unwrap());
        assert_eq!(
            dice.presentation().unwrap()["details"][1]["text"],
            "d20 · 17"
        );
        let math = Utility::Math {
            expression: "\\frac{1}{2}".into(),
            block: true,
        };
        assert!(
            math.presentation().unwrap()["mathml"]
                .as_str()
                .unwrap()
                .contains("<mfrac>")
        );
    }
}
