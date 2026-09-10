use crate::{
    utility::{Qr, Randomizer, Utility},
    Error, Text,
};
use serde_json::{json, Value};
use unicode_segmentation::UnicodeSegmentation;

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
                    let mut summary = groups
                        .iter()
                        .take(8)
                        .map(|g| {
                            format!(
                                "{}d{} · {}",
                                g.faces.len(),
                                g.sides,
                                g.faces.iter().map(|&v| u64::from(v)).sum::<u64>()
                            )
                        })
                        .collect::<Vec<_>>();
                    if groups.len() > 8 {
                        summary.push(format!("{} more groups", groups.len() - 8));
                    }
                    if groups.len() > 1 {
                        summary.push(format!(
                            "Total: {}",
                            groups
                                .iter()
                                .flat_map(|g| g.faces.iter())
                                .map(|&v| u64::from(v))
                                .sum::<u64>()
                        ));
                    }
                    json!({"kind":"dice","display":summary.join("\n"),"copy":self.body()?,"details":groups.iter().flat_map(|g|g.faces.iter().map(|face|plain(&format!("d{} · {}",g.sides,face)))).collect::<Result<Vec<_>,_>>()?,
                        "motion":{"kind":"dice","dice":groups.iter().flat_map(|g|g.faces.iter().map(|face|json!({"sides":g.sides,"face":face}))).take(6).collect::<Vec<_>>()}})
                }
                Randomizer::Pick {
                    category,
                    options,
                    selected,
                } => {
                    let visible = options
                        .iter()
                        .all(|t| t.spans().iter().all(|s| s.effects.reveal.is_none()));
                    let coin = category.as_deref() == Some("flip") && options.len() == 2;
                    let frames = options
                        .iter()
                        .take(12)
                        .map(|t| {
                            let mut graphemes = t.body().graphemes(true);
                            let mut label = graphemes.by_ref().take(48).collect::<String>();
                            if graphemes.next().is_some() {
                                label.push('…');
                            }
                            label
                        })
                        .collect::<Vec<_>>();
                    json!({"kind":"pick","display":category.as_deref().unwrap_or("Choice"),"rich":options[*selected as usize].presentation(),"details":options.iter().map(Text::presentation).collect::<Vec<_>>(),"selected":selected,
                        "motion":visible.then(||json!({"kind":if coin {"coin"}else{"choice"},"frames":frames,"selected":selected,"result":options[*selected as usize].body()}))})
                }
                Randomizer::Number { min, max, selected } => {
                    let span = i128::from(*max) - i128::from(*min);
                    let frames = [9, 2, 11, 5, 14, 0, 13, 4, 10, 1, 12, 6]
                        .map(|n| (i128::from(*min) + span * n / 15).to_string());
                    json!({"kind":"random","display":selected.to_string(),"copy":selected.to_string(),"alternate":format!("Between {min} and {max}"),
                        "motion":{"kind":"number","frames":frames,"result":selected.to_string()}})
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
        assert!(view["qr"]["payload"]
            .as_str()
            .unwrap()
            .contains("synthetic-secret"));
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
        assert!(math.presentation().unwrap()["mathml"]
            .as_str()
            .unwrap()
            .contains("<mfrac>"));
    }
    #[test]
    fn randomizer_motion_is_bounded_and_never_reveals_concealed_candidates() {
        let dice = Utility::Random(Randomizer::Dice {
            groups: vec![crate::utility::Dice {
                sides: 20,
                faces: vec![17; 40],
            }],
        });
        let before = dice.clone();
        let view = dice.presentation().unwrap();
        assert_eq!(view["motion"]["dice"].as_array().unwrap().len(), 6);
        assert_eq!(view["motion"]["dice"][0], json!({"sides":20,"face":17}));
        assert!(dice == before);
        let hidden = Utility::Random(Randomizer::Pick {
            category: None,
            options: vec![
                crate::parse("spoiler::Secret;", Default::default()).unwrap(),
                Text::plain("Visible", Default::default()).unwrap(),
            ],
            selected: 1,
        });
        assert!(hidden.presentation().unwrap()["motion"].is_null());
        let number = Utility::Random(Randomizer::Number {
            min: i64::MIN,
            max: i64::MAX,
            selected: 42,
        });
        let view = number.presentation().unwrap();
        assert_eq!(view["motion"]["result"], "42");
        assert_eq!(number.presentation().unwrap(), view);
        assert_eq!(view["motion"]["frames"].as_array().unwrap().len(), 12);
        for frame in view["motion"]["frames"].as_array().unwrap() {
            assert!(frame.as_str().unwrap().parse::<i64>().is_ok());
        }
    }
}
