use crate::{
    action::{Action, Change, Reference},
    structured::{Card, Construct, Id, ListMode},
    Error,
};
pub(crate) fn validate_shape(content: &Construct, at: u64) -> Result<(), Error> {
    if !matches!(
        content,
        Construct::Note(_)
            | Construct::Reminder(_)
            | Construct::Checklist(_)
            | Construct::Location(_)
    ) {
        return Err(Error::Invalid);
    }
    let created_at = if let Construct::Checklist(list) = content {
        if let ListMode::Recurring(rule) = &list.mode {
            if rule.anchor_at > at {
                return Err(Error::Invalid);
            }
            rule.anchor_at
        } else {
            at
        }
    } else {
        at
    };
    if let Construct::Location(value) = content {
        value.point.validate()?;
        return if value.label.body().len() <= 256
            && matches!(value.mode,crate::location::Mode::Live{device,..} if device!=[0;32])
        {
            Ok(())
        } else {
            Err(Error::Invalid)
        };
    }
    Card {
        id: [1; 32],
        creator: [1; 32],
        created_at,
        content: content.clone(),
    }
    .validate(Default::default())
}

pub(crate) fn validate_content(card: &Card, content: &Construct, at: u64) -> Result<(), Error> {
    if at < card.created_at {
        return Err(Error::Invalid);
    }
    match (&card.content, content) {
        (Construct::Note(_), Construct::Note(_))
        | (Construct::Location(_), Construct::Location(_))
        | (Construct::Reminder(_), Construct::Reminder(_)) => (),
        (Construct::Checklist(old), Construct::Checklist(new))
            if std::mem::discriminant(&old.mode) == std::mem::discriminant(&new.mode) => {}
        _ => return Err(Error::Invalid),
    }
    let mut edited = card.clone();
    edited.content = content.clone();
    if let Construct::Checklist(list) = content {
        if let ListMode::Recurring(rule) = &list.mode {
            if rule.anchor_at < card.created_at || rule.anchor_at > at {
                return Err(Error::Invalid);
            }
            edited.created_at = rule.anchor_at;
        }
    }
    edited.validate(Default::default())
}
pub(crate) fn validate_edit(
    action: &Action,
    card: &Card,
    content: &Construct,
    policy_id: Option<Id>,
    policy: Option<&Action>,
    parent: Option<&Action>,
) -> Result<(), Error> {
    validate_content(card, content, action.created_at)?;
    if let (Construct::Location(original), Construct::Location(value)) = (&card.content, content) {
        if action.actor != card.creator || policy_id.is_some() {
            return Err(Error::Invalid);
        }
        let previous = match parent {
            Some(parent) => match &parent.change {
                Change::Edit {
                    content: Construct::Location(value),
                    ..
                } => value,
                _ => return Err(Error::Invalid),
            },
            None => original,
        };
        return value.validate_update(original, previous, card.created_at, action.created_at);
    }
    let baseline = match (policy_id, policy) {
        (None, None) => &card.content,
        (Some(id), Some(policy))
            if policy.id()? == id && policy.card == action.card && policy.actor == card.creator =>
        {
            let Change::Editors { content, editors } = &policy.change else {
                return Err(Error::Invalid);
            };
            if action.actor != card.creator && editors.binary_search(&action.actor).is_err() {
                return Err(Error::Invalid);
            }
            content
        }
        _ => return Err(Error::Invalid),
    };
    if action.actor == card.creator {
        return Ok(());
    }
    if policy_id.is_none() {
        return Err(Error::Invalid);
    }
    let baseline = match (action.previous, parent) {
        (None, None) => baseline,
        (Some(id), Some(parent))
            if parent.id()? == id
                && parent.card == action.card
                && parent.register()? == action.register()? =>
        {
            let Change::Edit { content, .. } = &parent.change else {
                return Err(Error::Invalid);
            };
            content
        }
        _ => return Err(Error::Invalid),
    };
    match (baseline, content) {
        (Construct::Checklist(old), Construct::Checklist(new))
            if old.title == new.title
                && old.items == new.items
                && matches!(old.mode, ListMode::Recurring(_))
                && matches!(new.mode, ListMode::Recurring(_)) =>
        {
            Ok(())
        }
        _ => Err(Error::Invalid),
    }
}
pub fn at_revision(action: &Action, card: &Card, revision: Option<&Action>) -> Result<Card, Error> {
    let mut value = card.clone();
    match (action.revision, revision) {
        (None, None) => (),
        (Some(id), Some(revision))
            if revision.id()? == id && revision.card == Reference::of(card)? =>
        {
            let (Change::Edit { content, .. } | Change::Editors { content, .. }) = &revision.change
            else {
                return Err(Error::Invalid);
            };
            validate_content(card, content, revision.created_at)?;
            value.content = content.clone();
        }
        _ => return Err(Error::Invalid),
    }
    Ok(value)
}

/// Returns authoring syntax only when reparsing preserves the entire canonical value.
pub fn text_source(text: &crate::Text) -> Option<String> {
    use unicode_segmentation::UnicodeSegmentation;
    if !text.blocks().is_empty() || !text.mentions().is_empty() { return None; }
    fn literal(value: &str) -> String {
        let mut result = String::new();
        for c in value.chars() { if c.is_ascii_punctuation() { result.push('\\'); } result.push(c); }
        result
    }
    let offsets: Vec<_> = text.body().grapheme_indices(true).map(|(at,_)|at).chain(std::iter::once(text.body().len())).collect();
    let mut result = String::new();
    let mut at = 0;
    for span in text.spans() {
        let start = offsets[span.start as usize]; let end = offsets[span.end as usize];
        result.push_str(&literal(&text.body()[at..start]));
        let body = &text.body()[start..end];
        let e = &span.effects;
        let mut source = if e.code {
            let count = body.split(|c| c!='`').map(str::len).max().unwrap_or(0)+1;
            let marker = "`".repeat(count);
            let pad = body.starts_with(['`',' ']) || body.ends_with(['`',' ']);
            format!("{marker}{}{body}{}{marker}",if pad {" "} else {""},if pad {" "} else {""})
        } else { literal(body) };
        if let Some(url) = &e.link { source = format!("[{source}]({})",url.replace('(',"\\(").replace(')',"\\)")); }
        let mut tokens = Vec::new();
        for (enabled,token) in [(e.underline,"underline"),(e.mono,"mono"),(e.mark,"mark")] { if enabled { tokens.push(token.into()); } }
        if let Some(paint) = &e.paint { tokens.push(match paint {
            crate::Paint::Solid { color } => String::from(*color),
            crate::Paint::Gradient { stops } => stops.iter().map(|c|String::from(*c)).collect::<Vec<_>>().join("-"),
            crate::Paint::Rainbow => "rainbow".into(), crate::Paint::Theme => return None,
        }); }
        if let Some(size) = e.size { tokens.push(format!("{}{}",if size>0 {"big"} else {"small"},size.unsigned_abs())); }
        if let Some(reveal) = e.reveal { tokens.push(if reveal==crate::Reveal::Spoiler {"spoiler"} else {"scratch"}.into()); }
        if let Some(animation) = e.animation { tokens.push(format!("{animation:?}").to_ascii_lowercase()); }
        if !tokens.is_empty() { source = format!("{}::{source};",tokens.join("::")); }
        for (enabled,marker) in [(e.strike,"~~"),(e.italic,"*"),(e.bold,"**")] { if enabled { source = format!("{marker}{source}{marker}"); } }
        result.push_str(&source);
        at = end;
    }
    result.push_str(&literal(&text.body()[at..]));
    (crate::parse(&result,Default::default()).ok().as_ref()==Some(text)).then_some(result)
}

#[cfg(test)]
mod source_tests {
    use super::text_source;
    #[test]
    fn inline_source_round_trips_styles_code_links_unicode_and_redaction() {
        for source in ["plain **bold** and *italic*", "***bold and italic***", "a**partial**word", "underline::bold::Hello; red-blue::café;", "mono::green::status;", "mark::red::remember;", "small3::little; big2::large;", "spoiler::private; scratch::covered;", "wave::hello;", "`**literal**`", "`` `a` ``", "👩🏽‍💻 **שלום** e\u{301}", "[A link](https://example.test/a\\(b\\))", r"\*literal\* and \bold::literal;", "redact::SYNTHETIC_SECRET; **after**"] {
            let text = crate::parse(source,Default::default()).unwrap();
            let saved = text_source(&text).unwrap_or_else(||panic!("Cannot reconstruct {source}"));
            assert_eq!(crate::parse(&saved,Default::default()).unwrap().body(),text.body());
            assert!(!saved.contains("SYNTHETIC_SECRET"));
        }
        let heading = crate::parse("# Heading",Default::default()).unwrap();
        assert!(text_source(&heading).is_none());
        let value = crate::Text::from_runs(&[crate::Run{text:"word",effects:Default::default()},crate::Run{text:"part",effects:crate::Effects{underline:true,..Default::default()}}],Default::default()).unwrap();
        assert!(text_source(&value).is_none());
    }
}
