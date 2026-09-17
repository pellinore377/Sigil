use crate::{builder::literal, structured::{CardLimits,Construct}, data::Data, Error, Origin, Parsed};
use serde::Deserialize;
use serde_json::{json,Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Form {
    kind:String,
    #[serde(default)] title:String,
    #[serde(default)] mode:String,
    #[serde(default)] fields:Vec<String>,
    #[serde(default)] rows:Vec<Vec<String>>,
}
fn parsed(source:&str)->Result<crate::Draft,Error> {
    crate::parse_card(source,Origin {message:[1;32],creator:[2;32],created_at:1780000000,timezone:Some("UTC")},CardLimits::default())
}
pub fn source(input:&str)->Result<String,Error> {
    if input.len()>16384 {return Err(Error::Limit);}
    let f:Form=serde_json::from_str(input).map_err(|_|Error::Invalid)?;
    if f.fields.len()>8 || f.rows.len()>256 || f.rows.iter().any(|r|r.len()>4) {return Err(Error::Limit);}
    let field=|i:usize|f.fields.get(i).map(String::as_str).unwrap_or("");
    let title=literal(f.title.trim());
    let line=|i:usize|literal(field(i).trim());
    let mut body=match f.kind.as_str() {
        "Chart"=> {
            if !["bar","line","area","pie","donut","scatter"].contains(&f.mode.as_str()) {return Err(Error::Invalid);}
            let mut s=format!("chart::{}::{title}",f.mode);
            for r in &f.rows {if r.len()!=2 {return Err(Error::Invalid);}s.push_str(&format!("\n- {} = {}",literal(r[0].trim()),r[1].trim()));}
            s
        }
        "Diagram"=> {
            if !["flow","sequence","timeline","mindmap","org","state"].contains(&f.mode.as_str()) {return Err(Error::Invalid);}
            let mut s=format!("diagram::{}::{title}",f.mode);
            for r in &f.rows {
                if r.len()<2 {return Err(Error::Invalid);}
                let a=literal(r[0].trim());let b=literal(r[1].trim());let label=r.get(2).map(|s|literal(s.trim())).unwrap_or_default();
                s.push_str(&if f.mode=="timeline" {format!("\n- {a} = {b}")} else if f.mode=="sequence" {format!("\n- {a} {} {b}: {label}",if r.get(3).is_some_and(|s|s=="true") {"-->"}else{"->"})}else {format!("\n- {a} -> {b}{}",if label.is_empty(){String::new()}else{format!(" [{label}]")})});
            }s
        }
        "Recipe"=> {
            let mut s=format!("recipe::{title}");
            if !field(0).is_empty(){s.push_str(&format!("\nserves::{}",line(0)));}
            if !field(1).is_empty(){s.push_str(&format!("\ntime::{}",line(1)));}
            for section in ["ingredients","steps"] {s.push_str(&format!("\n{section}:"));for r in &f.rows {if r.len()!=2{return Err(Error::Invalid);}
if r[0]==section {s.push_str(&format!("\n- {}",literal(&r[1])));}}}
            s
        }
        "Recurring checklist"=> {
            if !["weekly","monthly","yearly"].contains(&f.mode.as_str()){return Err(Error::Invalid);}
            let mut s=format!("checklist::recurr::{}::{title}",f.mode);
            for r in &f.rows {if r.len()!=2{return Err(Error::Invalid);}s.push_str(&format!("\n{} {}",if r[1]=="true"{"-r-"}else{"-"},literal(&r[0])));}s
        }
        "Countdown"|"Elapsed time"=>format!("{}::{}::{title}",if f.kind=="Countdown"{"countdown"}else{"ago"},field(0).trim()),
        "Calculation"=>format!("calc::{}",line(0)),
        "Conversion"=>format!("convert::{} {}",line(0),line(1)),
        "Rating"=>format!("rate::{}/{}",field(0).trim(),field(1).trim()),
        "Progress"=>format!("progress::{}{}",field(0).trim(),if title.is_empty(){String::new()}else{format!("::{title}")}),
        "Color swatch"=>format!("swatch::{}",field(0).trim()),
        "Keyboard shortcut"=>format!("kbd::{}",field(0).split('+').map(|v|literal(v.trim())).collect::<Vec<_>>().join("+")),
        "Quote"=>format!("quote::{}::{}{}",line(0),if field(1).trim().is_empty(){String::new()}else{format!("{}::",line(1))},line(2)),
        "QR code"=>match f.mode.as_str() {
            "wifi"=>format!("qr::wifi::{}::{}",line(0),line(1)),
            "text"=>format!("qr::text::{}",line(0)),
            "link"=>format!("qr::{}",line(0)),
            _=>return Err(Error::Invalid),
        },
        "Math"|"ASCII art"=> {
            if field(0).lines().any(|l|l.trim()==";") {return Err(Error::Invalid);}
            format!("{}\n{}\n",if f.kind=="Math"{"math::block"}else{"art::"},field(0))
        }
        _=>return Err(Error::Invalid),
    };
    body.push(';');
    if body.len()>16384 {return Err(Error::Limit);}
    if !matches!(parsed(&body)?.content,Parsed::Card(_)){return Err(Error::Invalid);}
    Ok(body)
}
#[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Context {source:String,now:u64,timezone:String}
pub fn preview(input:&str)->Result<String,Error> {
    if input.len()>65536{return Err(Error::Limit); }

    let context=if input.starts_with('{') {Some(serde_json::from_str::<Context>(input).map_err(|_|Error::Invalid)?)}else{None};
    let source=context.as_ref().map_or(input,|c|c.source.as_str());
    if source.len()>16384{return Err(Error::Limit);}
    let origin=Origin {message:[1;32],creator:[2;32],created_at:context.as_ref().map_or(1780000000,|c|c.now),timezone:Some(context.as_ref().map_or("UTC",|c|c.timezone.as_str()))};
    let (ranges,_)=crate::composition::preview_ranges(source)?;
    let mut values=Vec::new();
    let mut consumed=0;
    let text=|s:&str|->Result<Value,Error> {let t=crate::parse(s,CardLimits::default().text)?;Ok(json!({"id":"preview","kind":"text","text":t.body(),"rich":t.presentation()}))};
    for range in ranges {
        let raw=&source[range.clone()];
        let value=if let Some(mut intent)=intent_preview(raw)? {
            intent["intent"]["start"]=json!(source[..range.start].encode_utf16().count());
            intent["intent"]["end"]=json!(source[..range.end].encode_utf16().count());
            Some(intent)
        } else if raw.starts_with("roll::") || raw.starts_with("pick::") {
            Some(random_preview(raw)?)
        } else {
            match crate::parse_card(raw,origin,CardLimits::default())?.content {
                Parsed::Card(card)=>Some(card_preview(&card,false)?),
                Parsed::Text(_)=>return Err(Error::Invalid),
            }
        };
        if let Some(value)=value {
            if consumed<range.start {values.push(text(&source[consumed..range.start])?);}
            values.push(value);consumed=range.end;
            if values.len()>64 {return Err(Error::Limit);}
        }
    }
    if consumed<source.len() {
        let rest=&source[consumed..];
        if let Some(mut intent)=intent_preview(rest.trim_start())? {
            intent["intent"]["ready"]=json!(false);
            intent["intent"]["start"]=json!(source[..source.len()-rest.trim_start().len()].encode_utf16().count());
            intent["intent"]["end"]=json!(source.encode_utf16().count());
            values.push(intent);
        } else {values.push(text(rest)?);}
    }
    if values.is_empty() {return Ok(text(source)?.to_string());}
    if values.len()==1 {return Ok(values.remove(0).to_string());}
    if values.len()>64 {return Err(Error::Limit);}
    Ok(json!({"id":"preview","kind":"composition","text":"","parts":values}).to_string())
}
fn intent_preview(source:&str)->Result<Option<Value>,Error> {
    let source=source.strip_suffix(';').unwrap_or(source);
    let (tool,body,language,forecast)=if let Some(rest)=source.strip_prefix("translate::") {
        let (language,body)=rest.split_once("::").unwrap_or((rest,""));
        ("Translation",body,language,false)
    } else if let Some(body)=source.strip_prefix("define::") {("Definition",body,"en",false)}
    else if let Some(body)=source.strip_prefix("weather::") {("Weather",body.strip_suffix("::forecast").unwrap_or(body),"en",body.ends_with("::forecast"))}
    else if let Some(body)=source.strip_prefix("@::") {("Contact",body,"",false)}
    else if let Some(body)=source.strip_prefix("qr::contact::") {("Contact QR",body,"",false)}
    else {return Ok(None)};
    let text=crate::parse(body,CardLimits::default().text)?;
    let presentation=text.presentation();
    let concealed=presentation.spans.iter().any(|span|span.effects.reveal.is_some());
    let text=if concealed {String::new()}else{text.body().trim().to_owned()};
    let ready=!text.is_empty() && (tool!="Translation" || crate::service::language(language));
    Ok(Some(json!({"id":"preview","kind":"intent_preview","text":"","intent":{"tool":tool,"text":text,"language":language,"forecast":forecast,"ready":ready,"start":0,"end":0}})))
}
fn random_preview(source:&str)->Result<Value,Error> {
    let source=source.strip_suffix(';').ok_or(Error::Invalid)?;
    let limits=CardLimits::default();
    if let Some(dice)=source.strip_prefix("roll::") {
        let groups=crate::utility::dice_plan(dice,limits)?;
        return Ok(json!({"id":"preview","kind":"randomizer_preview","text":dice,"randomizer":{"kind":"dice","sides":groups.into_iter().flat_map(|(count,sides)|vec![sides;count]).collect::<Vec<_>>()}}));
    }
    let value=source.strip_prefix("pick::").ok_or(Error::Invalid)?;
    let (kind,label)=if let Some(range)=value.strip_prefix("number::") {
        let at=range.get(1..).ok_or(Error::Invalid)?.find('-').ok_or(Error::Invalid)?+1;
        let min=range[..at].parse::<i64>().map_err(|_|Error::Invalid)?;
        let max=range[at+1..].parse::<i64>().map_err(|_|Error::Invalid)?;
        if !(1..=u64::MAX as i128).contains(&(i128::from(max)-i128::from(min)+1)){return Err(Error::Invalid);}
        ("number",format!("{min}–{max}"))
    } else {
        let category=crate::utility::category(value);
        let options=category.map(|v|v.to_vec()).unwrap_or_else(||crate::utility::choices(value));
        if options.len()>limits.items{return Err(Error::Limit);}
        let options=options.into_iter().map(|v|crate::parse(v.trim(),limits.text).map(|t|t.body().to_owned())).collect::<Result<std::collections::BTreeSet<_>,_>>()?;
        let count=options.iter().filter(|v|!v.trim().is_empty()).count();
        if count==0{return Err(Error::Invalid);}
        (if value=="flip" {"coin"}else{"cards"},if value=="flip" {"Heads or tails".into()}else{format!("{count} choices")})
    };
    Ok(json!({"id":"preview","kind":"randomizer_preview","text":label,"randomizer":{"kind":kind,"sides":[]}}))
}

fn card_preview(card:&crate::structured::Card,random:bool)->Result<Value,Error> {
    let mut value=json!({"id":"preview","kind":"card","text":""});
    match &card.content {
        Construct::Data(Data::Chart(v))=>value["chart"]=v.presentation()?,
        Construct::Data(Data::Diagram(v))=>value["diagram"]=v.presentation()?,
        Construct::Data(Data::Recipe(v))=>value["recipe"]=v.presentation(None)?,
        Construct::Data(Data::Table(v))=>value["table"]=json!(v.presentation()),
        Construct::Utility(v)=>{if !random && matches!(v,crate::utility::Utility::Random(_)){return Err(Error::Invalid);}value["utility"]=v.presentation()?;}
        Construct::Note(v)=>{value["kind"]=json!("note");value["rich"]=json!(v.text.presentation());value["text"]=json!(v.text.body());}
        Construct::Checklist(v)=>{value["kind"]=json!(if matches!(v.mode,crate::structured::ListMode::Task){"task"}else{"checklist"});value["text"]=json!(v.title.body());value["rich"]=json!(v.title.presentation());value["items"]=Value::Array(v.items.iter().enumerate().map(|(i,item)|json!({"id":i.to_string(),"text":item.text.body(),"rich":item.text.presentation(),"checked":item.checked,"enabled":false})).collect());}
        Construct::Poll(v)=>{value["kind"]=json!("poll");value["text"]=json!(v.question.body());value["rich"]=json!(v.question.presentation());value["multiple"]=json!(!matches!(v.selection,crate::structured::Selection::Single));value["items"]=Value::Array(v.options.iter().enumerate().map(|(i,item)|json!({"id":i.to_string(),"text":item.text.body(),"rich":item.text.presentation(),"checked":false,"enabled":false})).collect());}
        Construct::Countdown(v)|Construct::Ago(v)|Construct::Reminder(v)=>{value["kind"]=json!(match &card.content {Construct::Countdown(_)=>"countdown",Construct::Ago(_)=>"ago",_=>"reminder"});value["text"]=json!(v.text.body());value["at"]=json!(v.at);}
        Construct::Timer(v)=>{value["kind"]=json!("timer");value["text"]=json!("Timer");value["at"]=json!(v.ends_at);value["started_at"]=json!(v.started_at);}
        _=>return Err(Error::Invalid),
    }
    Ok(value)
}

/// Explicit local submission for the design workbench, never an editor preview.
pub fn playground(input:&str)->Result<String,Error> {
    if input.len()>65536 {return Err(Error::Limit);}
    let c:Context=serde_json::from_str(input).map_err(|_|Error::Invalid)?;
    if c.source.len()>16384 {return Err(Error::Limit);}
    let doc=crate::composition::parse(&c.source,Origin {message:[1;32],creator:[2;32],created_at:c.now,timezone:Some(&c.timezone)},CardLimits::default(),None)?.content;
    let text=|v:crate::Text|json!({"id":"preview","kind":"text","text":v.body(),"rich":v.presentation()});
    let values=match doc {
        crate::Document::Text(v)=>vec![text(v)],
        crate::Document::Card(v)=>vec![card_preview(&v,true)?],
        crate::Document::Composition(v)=>v.parts.into_iter().map(|part|match part {crate::composition::Part::Text(v)=>Ok(text(v)),crate::composition::Part::Card(v)=>card_preview(&v,true)}).collect::<Result<Vec<_>,Error>>()?,
        _=>return Err(Error::Invalid),
    };
    serde_json::to_string(&values).map_err(|_|Error::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form(kind:&str,mode:&str,fields:&[&str],rows:&[&[&str]]) -> String {
        source(&json!({"kind":kind,"mode":mode,"title":"Example","fields":fields,"rows":rows}).to_string()).unwrap_or_else(|_|panic!("Invalid {kind} {mode}"))
    }
    #[test]
    fn provider_intents_are_local_concealment_safe_and_preserve_utf16_ranges() {
        let source="🙂\ntranslate::es::redact::PRIVATE; Hello;\nweather::Seattle::forecast;";
        let value=preview(source).unwrap();
        assert!(!value.contains("PRIVATE"));
        let value:Value=serde_json::from_str(&value).unwrap();
        let intents=value["parts"].as_array().unwrap().iter().filter_map(|v|v.get("intent")).collect::<Vec<_>>();
        assert_eq!(intents.len(),2);
        assert_eq!(intents[0]["tool"],"Translation");
        assert_eq!(intents[0]["start"],3);
        assert_eq!(intents[0]["language"],"es");
        assert_eq!(intents[1]["forecast"],true);
        for source in ["spoiler::weather::PRIVATE;;","`define::PRIVATE;`"] {
            assert!(!preview(source).unwrap().contains("intent_preview"));
        }
        let hidden=preview("translate::es::spoiler::PRIVATE;;").unwrap();
        assert!(!hidden.contains("PRIVATE"));
        let hidden:Value=serde_json::from_str(&hidden).unwrap();
        let intent=hidden.get("intent").or_else(||hidden["parts"].as_array().and_then(|parts|parts.iter().find_map(|part|part.get("intent")))).unwrap();
        assert_eq!(intent["ready"],false);
        for source in ["translate::","define::","weather::","@::"] {
            let value:Value=serde_json::from_str(&preview(source).unwrap()).unwrap();
            assert_eq!(value["kind"],"intent_preview");
            assert_eq!(value["intent"]["ready"],false);
        }
        assert_eq!(serde_json::from_str::<Value>(&preview("qr::contact::@sam:example.test;").unwrap()).unwrap()["intent"]["tool"],"Contact QR");
    }
    #[test]
    fn typed_preview_uses_composition_boundaries_without_resolving_results() {
        let source="wave::Hi;\n\nroll::2d6,1d20;\n\npick::flip;\n\npick::redact::HIDDEN;,blue;\n\ncalc::2+3;";
        let value=preview(source).unwrap();
        assert_eq!(value,preview(source).unwrap());
        assert!(!value.contains("HIDDEN"));
        let value:Value=serde_json::from_str(&value).unwrap();
        let parts=value["parts"].as_array().unwrap();
        assert_eq!(parts.iter().filter(|v|v["kind"]=="randomizer_preview").count(),3);
        let dice=parts.iter().find(|v|v["randomizer"]["kind"]=="dice").unwrap();
        assert_eq!(dice["randomizer"]["sides"],json!([6,6,20]));
        for part in parts.iter().filter(|v|v["kind"]=="randomizer_preview") {
            assert!(part.get("utility").is_none());
            assert!(part["randomizer"].get("selected").is_none());
            assert!(part["randomizer"].get("result").is_none());
        }
        let concealed=preview("redact::roll::2d6;; `pick::flip;`").unwrap();
        assert!(!concealed.contains("randomizer_preview"));
        assert!(!preview("roll::2d6").unwrap().contains("randomizer_preview"));
        assert!(preview("roll::257d6;").is_err());
        assert!(preview("pick::number::4-2;").is_err());
    }
    #[test]
    fn graphical_forms_produce_valid_cards_and_previews() {
        for mode in ["bar","line","area","pie","donut","scatter"] {
            let s=form("Chart",mode,&[],&[&["1","1.5"],&["2","2.5"]]);assert!(preview(&s).is_ok(),"{s}");
        }
        for mode in ["flow","sequence","timeline","mindmap","org","state"] {
            let s=form("Diagram",mode,&[],&[&["Start","End","go"]]);assert!(preview(&s).is_ok(),"{s}");
        }
        for (kind,fields) in [
            ("Recipe",vec!["4","25 min"]),("Countdown",vec!["2027-07-05T09:30:00"]),("Elapsed time",vec!["2026-01-01T09:30:00"]),
            ("Calculation",vec!["(2 + 3) * 4"]),("Conversion",vec!["2.5","miles"]),("Rating",vec!["3.5","5"]),
            ("Progress",vec!["40"]),("Color swatch",vec!["#8038ba"]),("Keyboard shortcut",vec!["Ctrl+Shift+P"]),
            ("Quote",vec!["Sam","Book","Hello; world"]),("Math",vec!["\\frac{1}{2}"]),("ASCII art",vec![" /\\\n/__\\"])
        ] {
            let rows:Vec<&[&str]>=if kind=="Recipe"{vec![&["ingredients","200g pasta"],&["steps","Boil water"]]}else{vec![]};
            let s=form(kind,"",&fields,&rows);assert!(preview(&s).is_ok(),"{s}");
        }
        for mode in ["weekly","monthly","yearly"] {let s=form("Recurring checklist",mode,&[],&[&["Milk; eggs","true"]]);assert!(preview(&s).is_ok(),"{s}");}
        for (mode,fields) in [("text",vec!["Hello; world"]),("link",vec!["https://example.test/a?q=2"]),("wifi",vec!["Network","p:a;s"])] {let s=form("QR code",mode,&fields,&[]);assert!(preview(&s).is_ok(),"{s}");}
    }
    #[test]
    fn contextual_previews_keep_timezone_and_redaction_semantics() {
        let at=|zone:&str| {let v:Value=serde_json::from_str(&preview(&json!({"source":"countdown::2027-07-05T09:30:00::Meet;","now":1780000000u64,"timezone":zone}).to_string()).unwrap()).unwrap();v["at"].as_u64().unwrap()};
        assert_eq!(at("America/New_York")-at("UTC"),14400);
        let v=preview(&json!({"source":"redact::SYNTHETIC_SECRET;","now":1780000000u64,"timezone":"UTC"}).to_string()).unwrap();
        assert!(!v.contains("SYNTHETIC_SECRET"));assert!(v.contains("[REDACTED]"));
    }
    #[test]
    fn preview_does_not_resolve_randomizers_and_invalid_rows_are_rejected() {
        for s in ["roll::d6;","calc::nope;"] {assert!(preview(s).is_err());}
        for value in ["NaN","infinity","2;note::injected"] {assert!(source(&json!({"kind":"Chart","mode":"bar","title":"Test","rows":[["A",value]]}).to_string()).is_err());}
        let s=form("Recipe","",&[],&[&["ingredients","bold::plain;"],&["steps","Do it"]]);
        let view:Value=serde_json::from_str(&preview(&s).unwrap()).unwrap();
        assert_eq!(view["recipe"]["ingredients"][0]["text"],"bold::plain;");
    }
}
    #[test]
    fn explicit_playground_submission_preserves_composition_and_redaction() {
        let request=json!({"source":"roll::1d6;\n\nredact::private; A caption","now":1780000000u64,"timezone":"UTC"});
        let parts:Value=serde_json::from_str(&playground(&request.to_string()).unwrap()).unwrap();
        assert_eq!(parts.as_array().unwrap().len(),2);
        assert!(parts[0]["utility"]["motion"].is_object());
        assert!(!parts.to_string().contains("private"));
        assert!(parts[1]["text"].as_str().unwrap().contains("A caption"));
        assert!(preview("roll::d6;").is_err());
    }
