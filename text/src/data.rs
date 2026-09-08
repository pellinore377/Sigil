use crate::{numeric::Number, structured::CardLimits, Error, Text};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Data {
    Chart(Chart),
    Diagram(Diagram),
    Table(Table),
    Recipe(Recipe),
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Pie,
    Donut,
    Bar,
    Line,
    Area,
    Scatter,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: Text,
    pub points: Vec<Point>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub label: Text,
    pub x: Option<Number>,
    pub y: Number,
    pub percent: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramKind {
    Flow,
    Sequence,
    Timeline,
    Mindmap,
    Org,
    State,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Default,
    Decision,
    Process,
    Rounded,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub label: Text,
    pub shape: Shape,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: u16,
    pub to: u16,
    pub label: Text,
    pub dashed: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagram {
    pub kind: DiagramKind,
    pub title: Text,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub entries: Vec<(Text, Text)>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub columns: Vec<Text>,
    pub rows: Vec<Vec<Text>>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub title: Text,
    pub serves: Option<u16>,
    pub seconds: Option<u64>,
    pub ingredients: Vec<Text>,
    pub steps: Vec<Text>,
}
fn nonempty(text: &Text) -> Result<(), Error> {
    if text.body().trim().is_empty() {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}
impl Data {
    pub fn texts(&self) -> Vec<&Text> {
        match self {
            Self::Chart(v) => std::iter::once(&v.title)
                .chain(v.points.iter().map(|p| &p.label))
                .collect(),
            Self::Diagram(v) => std::iter::once(&v.title)
                .chain(v.nodes.iter().map(|n| &n.label))
                .chain(v.edges.iter().map(|e| &e.label))
                .chain(v.entries.iter().flat_map(|(a, b)| [a, b]))
                .collect(),
            Self::Table(v) => v.columns.iter().chain(v.rows.iter().flatten()).collect(),
            Self::Recipe(v) => std::iter::once(&v.title)
                .chain(&v.ingredients)
                .chain(&v.steps)
                .collect(),
        }
    }
    pub fn validate(&self, limits: CardLimits) -> Result<(), Error> {
        limits.validate()?;
        match self {
            Self::Chart(v) => {
                nonempty(&v.title)?;
                if v.points.is_empty() || v.points.len() > limits.items {
                    return Err(Error::Limit);
                }
                let circular = matches!(v.kind, ChartKind::Pie | ChartKind::Donut);
                for point in &v.points {
                    nonempty(&point.label)?;
                    if point.x.is_some() != (v.kind == ChartKind::Scatter)
                        || (circular && point.y.value() < 0.0)
                    {
                        return Err(Error::Invalid);
                    }
                }
                if circular && !v.points.iter().any(|p| p.y.value() > 0.0) {
                    return Err(Error::Invalid);
                }
            }
            Self::Diagram(v) => {
                nonempty(&v.title)?;
                if v.nodes.len() > limits.items
                    || v.edges.len() > limits.items
                    || v.entries.len() > limits.items
                {
                    return Err(Error::Limit);
                }
                if v.kind == DiagramKind::Timeline {
                    if v.entries.is_empty() || !v.nodes.is_empty() || !v.edges.is_empty() {
                        return Err(Error::Invalid);
                    }
                    for (date, label) in &v.entries {
                        nonempty(date)?;
                        nonempty(label)?;
                    }
                } else {
                    if v.nodes.is_empty() || v.edges.is_empty() || !v.entries.is_empty() {
                        return Err(Error::Invalid);
                    }
                    let mut labels = BTreeSet::new();
                    for node in &v.nodes {
                        nonempty(&node.label)?;
                        if !labels.insert(node.label.body()) {
                            return Err(Error::Invalid);
                        }
                    }
                    let mut parents = vec![0usize; v.nodes.len()];
                    let mut outgoing = vec![Vec::new(); v.nodes.len()];
                    for edge in &v.edges {
                        if edge.from as usize >= v.nodes.len() || edge.to as usize >= v.nodes.len()
                        {
                            return Err(Error::Invalid);
                        }
                        if edge.dashed && v.kind != DiagramKind::Sequence {
                            return Err(Error::Invalid);
                        }
                        parents[edge.to as usize] += 1;
                        outgoing[edge.from as usize].push(edge.to as usize);
                    }
                    if matches!(v.kind, DiagramKind::Org | DiagramKind::Mindmap) {
                        if parents.iter().any(|&n| n > 1)
                            || parents.iter().filter(|&&n| n == 0).count() != 1
                        {
                            return Err(Error::Invalid);
                        }
                        let mut queue: VecDeque<_> = parents
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &n)| (n == 0).then_some(i))
                            .collect();
                        let mut visited = 0;
                        while let Some(node) = queue.pop_front() {
                            visited += 1;
                            for &child in &outgoing[node] {
                                parents[child] -= 1;
                                if parents[child] == 0 {
                                    queue.push_back(child);
                                }
                            }
                        }
                        if visited != v.nodes.len() {
                            return Err(Error::Invalid);
                        }
                    }
                }
            }
            Self::Table(v) => {
                if v.columns.is_empty()
                    || v.columns.len() > limits.options
                    || v.rows.is_empty()
                    || v.rows.len() > limits.items
                {
                    return Err(Error::Limit);
                }
                if v.columns.iter().all(|t| t.body().trim().is_empty()) {
                    return Err(Error::Invalid);
                }
                for row in &v.rows {
                    if row.len() != v.columns.len()
                        || row.iter().all(|v| v.body().trim().is_empty())
                    {
                        return Err(Error::Invalid);
                    }
                }
            }
            Self::Recipe(v) => {
                nonempty(&v.title)?;
                if v.serves == Some(0) || v.seconds == Some(0) {
                    return Err(Error::Invalid);
                }
                if v.ingredients.is_empty()
                    || v.steps.is_empty()
                    || v.ingredients.len() + v.steps.len() > limits.items
                {
                    return Err(Error::Limit);
                }
                for text in v.ingredients.iter().chain(&v.steps) {
                    nonempty(text)?;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn body(&self) -> String {
        match self {
            Self::Chart(v) => std::iter::once(format!("Chart: {}", v.title.body()))
                .chain(v.points.iter().map(|p| {
                    format!(
                        "{} = {}{}",
                        p.label.body(),
                        p.y.as_str(),
                        if p.percent { "%" } else { "" }
                    )
                }))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Diagram(v) => std::iter::once(format!("Diagram: {}", v.title.body()))
                .chain(v.edges.iter().map(|e| {
                    format!(
                        "{} {} {}{}",
                        v.nodes[e.from as usize].label.body(),
                        if e.dashed { "-->" } else { "->" },
                        v.nodes[e.to as usize].label.body(),
                        if e.label.body().is_empty() {
                            String::new()
                        } else {
                            format!(": {}", e.label.body())
                        }
                    )
                }))
                .chain(
                    v.entries
                        .iter()
                        .map(|(a, b)| format!("{} = {}", a.body(), b.body())),
                )
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Table(v) => std::iter::once(&v.columns)
                .chain(&v.rows)
                .map(|row| row.iter().map(Text::body).collect::<Vec<_>>().join(" | "))
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Recipe(v) => {
                let mut lines = vec![format!("Recipe: {}", v.title.body())];
                if let Some(n) = v.serves {
                    lines.push(format!("Serves: {n}"));
                }
                if let Some(n) = v.seconds {
                    lines.push(format!("Time: {n} seconds"));
                }
                lines.push("Ingredients:".into());
                lines.extend(v.ingredients.iter().map(|t| format!("• {}", t.body())));
                lines.push("Steps:".into());
                lines.extend(
                    v.steps
                        .iter()
                        .enumerate()
                        .map(|(i, t)| format!("{}. {}", i + 1, t.body())),
                );
                lines.join("\n")
            }
        }
    }
    pub fn html(&self) -> Result<String, Error> {
        self.validate(Default::default())?;
        if let Self::Table(v) = self {
            let mut out = String::from("<table><thead><tr>");
            for cell in &v.columns {
                out.push_str("<th>");
                out.push_str(&cell.html());
                out.push_str("</th>");
            }
            out.push_str("</tr></thead><tbody>");
            for row in &v.rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    out.push_str(&cell.html());
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
            Ok(out)
        } else {
            Ok(Text::plain(&self.body(), Default::default())?.html())
        }
    }
}

pub(crate) fn split(source: &str, operator: &str) -> Vec<usize> {
    let mut result = Vec::new();
    for (event, range) in pulldown_cmark::Parser::new(source).into_offset_iter() {
        if let pulldown_cmark::Event::Text(text) = event {
            if source[range.clone()] == *text {
                for (offset, _) in text.match_indices(operator) {
                    let at = range.start + offset;
                    if source.as_bytes()[..at]
                        .iter()
                        .rev()
                        .take_while(|&&b| b == b'\\')
                        .count()
                        % 2
                        == 0
                    {
                        result.push(at);
                    }
                }
            }
        }
    }
    result
}
fn cells(source: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    for at in split(source, "|") {
        result.push(source[start..at].trim());
        start = at + 1;
    }
    result.push(source[start..].trim());
    result
}
pub(crate) fn parse(lines: &[&str], limits: CardLimits) -> Result<Data, Error> {
    let (keyword, header) = lines[0].split_once("::").ok_or(Error::Invalid)?;
    let text = |s: &str| crate::parse(s, limits.text);
    if lines.len() > limits.items + 5 {
        return Err(Error::Limit);
    }
    let rows = || {
        lines[1..]
            .iter()
            .map(|line| line.strip_prefix("- ").ok_or(Error::Invalid))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(match keyword {
        "table" => {
            let columns = cells(header)
                .into_iter()
                .map(text)
                .collect::<Result<Vec<_>, _>>()?;
            let mut values = Vec::new();
            for row in rows()? {
                let mut row = cells(row)
                    .into_iter()
                    .map(text)
                    .collect::<Result<Vec<_>, _>>()?;
                if row.len() > columns.len() {
                    return Err(Error::Invalid);
                }
                while row.len() < columns.len() {
                    row.push(text("")?);
                }
                values.push(row);
            }
            Data::Table(Table {
                columns,
                rows: values,
            })
        }
        "chart" => {
            let (kind, title) = header.split_once("::").ok_or(Error::Invalid)?;
            let kind = match kind {
                "pie" => ChartKind::Pie,
                "donut" => ChartKind::Donut,
                "bar" => ChartKind::Bar,
                "line" => ChartKind::Line,
                "area" => ChartKind::Area,
                "scatter" => ChartKind::Scatter,
                _ => return Err(Error::Invalid),
            };
            let mut points = Vec::new();
            for row in rows()? {
                let at = *split(row, "=").last().ok_or(Error::Invalid)?;
                let label = text(row[..at].trim())?;
                let value = row[at + 1..].trim();
                let percent = value.ends_with('%');
                let y =
                    Number::try_from(value.strip_suffix('%').unwrap_or(value).trim().to_owned())?;
                let x = if kind == ChartKind::Scatter {
                    Some(Number::try_from(label.body().to_owned())?)
                } else {
                    None
                };
                points.push(Point {
                    label,
                    x,
                    y,
                    percent,
                });
            }
            Data::Chart(Chart {
                kind,
                title: text(title)?,
                points,
            })
        }
        "diagram" => {
            let (kind, title) = header.split_once("::").ok_or(Error::Invalid)?;
            let kind = match kind {
                "flow" => DiagramKind::Flow,
                "sequence" => DiagramKind::Sequence,
                "timeline" => DiagramKind::Timeline,
                "mindmap" => DiagramKind::Mindmap,
                "org" => DiagramKind::Org,
                "state" => DiagramKind::State,
                _ => return Err(Error::Invalid),
            };
            let mut value = Diagram {
                kind,
                title: text(title)?,
                nodes: Vec::new(),
                edges: Vec::new(),
                entries: Vec::new(),
            };
            let mut names = BTreeMap::new();
            for row in rows()? {
                if kind == DiagramKind::Timeline {
                    let at = *split(row, "=").last().ok_or(Error::Invalid)?;
                    value
                        .entries
                        .push((text(row[..at].trim())?, text(row[at + 1..].trim())?));
                    continue;
                }
                let dashed = !split(row, "-->").is_empty();
                let separator = if dashed { "-->" } else { "->" };
                let positions = split(row, separator);
                if positions.len() != 1 {
                    return Err(Error::Invalid);
                }
                let at = positions[0];
                let left = row[..at].trim();
                let mut right = row[at + separator.len()..].trim();
                let label = if kind == DiagramKind::Sequence {
                    let at = *split(right, ":").first().ok_or(Error::Invalid)?;
                    let label = text(right[at + 1..].trim())?;
                    right = right[..at].trim();
                    label
                } else if right.ends_with(']') {
                    if let Some(at) = right.rfind(" [") {
                        let label = text(&right[at + 2..right.len() - 1])?;
                        right = right[..at].trim();
                        label
                    } else {
                        text("")?
                    }
                } else {
                    text("")?
                };
                let mut ids = Vec::new();
                for raw in [left, right] {
                    let (raw, shape) = if raw.starts_with('{') && raw.ends_with('}') {
                        (&raw[1..raw.len() - 1], Shape::Decision)
                    } else if raw.starts_with('[') && raw.ends_with(']') {
                        (&raw[1..raw.len() - 1], Shape::Process)
                    } else if raw.starts_with('(') && raw.ends_with(')') {
                        (&raw[1..raw.len() - 1], Shape::Rounded)
                    } else {
                        (raw, Shape::Default)
                    };
                    let label = text(raw)?;
                    let id = if let Some(&id) = names.get(label.body()) {
                        if value.nodes[id as usize].shape != shape && shape != Shape::Default {
                            return Err(Error::Invalid);
                        }
                        id
                    } else {
                        if value.nodes.len() >= limits.items {
                            return Err(Error::Limit);
                        }
                        let id = value.nodes.len() as u16;
                        names.insert(label.body().to_owned(), id);
                        value.nodes.push(Node { label, shape });
                        id
                    };
                    ids.push(id);
                }
                value.edges.push(Edge {
                    from: ids[0],
                    to: ids[1],
                    label,
                    dashed,
                });
            }
            Data::Diagram(value)
        }
        "recipe" => {
            let mut value = Recipe {
                title: text(header)?,
                serves: None,
                seconds: None,
                ingredients: Vec::new(),
                steps: Vec::new(),
            };
            let mut section = 0;
            for line in &lines[1..] {
                match *line {
                    "ingredients:" if section == 0 => section = 1,
                    "steps:" if section == 1 => section = 2,
                    line if section == 0
                        && line.starts_with("serves::")
                        && value.serves.is_none() =>
                    {
                        value.serves = Some(line[8..].parse().map_err(|_| Error::Invalid)?)
                    }
                    line if section == 0
                        && line.starts_with("time::")
                        && value.seconds.is_none() =>
                    {
                        value.seconds = Some(crate::time::duration(&line[6..])?)
                    }
                    line => {
                        let content = text(line.strip_prefix("- ").ok_or(Error::Invalid)?)?;
                        match section {
                            1 => value.ingredients.push(content),
                            2 => value.steps.push(content),
                            _ => return Err(Error::Invalid),
                        }
                    }
                }
            }
            if section != 2 {
                return Err(Error::Invalid);
            }
            Data::Recipe(value)
        }
        _ => return Err(Error::Invalid),
    })
}
impl Chart {
    pub fn shares(&self) -> Result<Vec<f64>, Error> {
        Data::Chart(self.clone()).validate(Default::default())?;
        if !matches!(self.kind, ChartKind::Pie | ChartKind::Donut) {
            return Err(Error::Invalid);
        }
        let max = self.points.iter().map(|p| p.y.value()).fold(0.0, f64::max);
        let sum = self.points.iter().map(|p| p.y.value() / max).sum::<f64>();
        Ok(self
            .points
            .iter()
            .map(|p| (p.y.value() / max) / sum)
            .collect())
    }
}
