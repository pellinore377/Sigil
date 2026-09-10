use crate::{Presentation, Text, data::Table, numeric::Number};
use serde::Serialize;

#[derive(Serialize)]
pub struct View<'a> {
    pub columns: Vec<Presentation<'a>>,
    pub rows: Vec<Vec<Presentation<'a>>>,
    pub numeric_order: Vec<Option<Vec<usize>>>,
    pub copy_rows: Vec<Option<String>>,
    pub copy_table: Option<String>,
}

fn visible(text: &Text) -> bool {
    text.spans()
        .iter()
        .all(|span| span.effects.reveal.is_none())
}
fn copy_row(row: &[Text]) -> Option<String> {
    row.iter().all(visible).then(|| {
        row.iter()
            .map(|cell| {
                let value = cell.body();
                if value.contains(['\t', '\n', '\r', '"']) {
                    format!("\"{}\"", value.replace('"', "\"\""))
                } else {
                    value.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\t")
    })
}
impl Table {
    pub fn presentation(&self) -> View<'_> {
        let copy_rows: Vec<_> = self.rows.iter().map(|row| copy_row(row)).collect();
        let copy_table = copy_row(&self.columns).and_then(|header| {
            copy_rows
                .iter()
                .cloned()
                .collect::<Option<Vec<_>>>()
                .map(|rows| {
                    std::iter::once(header)
                        .chain(rows)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
        });
        let numeric_order = (0..self.columns.len())
            .map(|column| {
                if !visible(&self.columns[column]) {
                    return None;
                }
                let numbers = self
                    .rows
                    .iter()
                    .map(|row| {
                        let text = row.get(column)?;
                        visible(text)
                            .then(|| Number::try_from(text.body().trim().to_owned()).ok())
                            .flatten()
                    })
                    .collect::<Option<Vec<_>>>()?;
                let mut order: Vec<_> = (0..numbers.len()).collect();
                order.sort_by(|&a, &b| numbers[a].value().total_cmp(&numbers[b].value()));
                Some(order)
            })
            .collect();
        View {
            columns: self.columns.iter().map(Text::presentation).collect(),
            rows: self
                .rows
                .iter()
                .map(|r| r.iter().map(Text::presentation).collect())
                .collect(),
            numeric_order,
            copy_rows,
            copy_table,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_view_order_and_copy_preserve_the_authored_table_and_hide_secrets() {
        let plain = |s| Text::plain(s, Default::default()).unwrap();
        let mut table = Table {
            columns: vec![plain("Name"), plain("Count")],
            rows: vec![
                vec![plain("👋\tA"), plain("10")],
                vec![plain("B\n\"C\""), plain("2")],
                vec![plain("D"), plain("2")],
            ],
        };
        let before = serde_json::to_vec(&table).unwrap();
        let view = table.presentation();
        assert_eq!(view.numeric_order, vec![None, Some(vec![1, 2, 0])]);
        assert_eq!(
            view.copy_table.as_deref(),
            Some("Name\tCount\n\"👋\tA\"\t10\n\"B\n\"\"C\"\"\"\t2\nD\t2")
        );
        assert_eq!(serde_json::to_vec(&table).unwrap(), before);
        table.rows[1][1] = crate::parse("spoiler::2;", Default::default()).unwrap();
        let view = table.presentation();
        assert!(view.numeric_order[1].is_none());
        assert!(view.copy_rows[1].is_none());
        assert!(view.copy_table.is_none());
        table.rows[1][1] = plain("NaN");
        assert!(table.presentation().numeric_order[1].is_none());
    }
}
