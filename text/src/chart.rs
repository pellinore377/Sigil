use crate::{
    Error,
    data::{Chart, ChartKind},
    numeric::Number,
};
use serde_json::{Value, json};

fn axis(values: &[f64]) -> (Vec<f64>, f64, Vec<String>) {
    let scale = values.iter().map(|v| v.abs()).fold(1.0, f64::max);
    let normalized: Vec<_> = values.iter().map(|v| v / scale).collect();
    let mut min = normalized.iter().copied().fold(0.0, f64::min);
    let mut max = normalized.iter().copied().fold(0.0, f64::max);
    if min == max {
        min = -1.0;
        max = 1.0;
    }
    let range = max - min;
    let ticks = (0..=4)
        .map(|i| {
            Number::new((min + range * f64::from(i) / 4.0).clamp(min, max) * scale)
                .unwrap()
                .as_str()
                .to_owned()
        })
        .collect();
    (
        normalized.iter().map(|v| (v - min) / range).collect(),
        -min / range,
        ticks,
    )
}
impl Chart {
    pub fn presentation(&self) -> Result<Value, Error> {
        crate::data::Data::Chart(self.clone()).validate(Default::default())?;
        let circular = matches!(self.kind, ChartKind::Pie | ChartKind::Donut);
        let shares = if circular {
            self.shares()?
        } else {
            vec![0.0; self.points.len()]
        };
        let (ys, zero, y_ticks) =
            axis(&self.points.iter().map(|p| p.y.value()).collect::<Vec<_>>());
        let (xs, _, x_ticks) = if self.kind == ChartKind::Scatter {
            axis(
                &self
                    .points
                    .iter()
                    .map(|p| p.x.as_ref().unwrap().value())
                    .collect::<Vec<_>>(),
            )
        } else {
            (
                (0..self.points.len())
                    .map(|i| (i as f64 + 0.5) / self.points.len() as f64)
                    .collect(),
                0.0,
                Vec::new(),
            )
        };
        let horizontal = self.kind == ChartKind::Bar
            && (self.points.len() > 6
                || self
                    .points
                    .iter()
                    .any(|p| p.label.body().chars().count() > 12));
        let copy_data = self
            .points
            .iter()
            .all(|p| p.label.spans().iter().all(|s| s.effects.reveal.is_none()))
            .then(|| {
                self.points
                    .iter()
                    .map(|p| {
                        format!(
                            "\"{}\"\t{}{}{}",
                            p.label.body().replace('"', "\"\""),
                            p.x.as_ref()
                                .map(|v| format!("{}\t", v.as_str()))
                                .unwrap_or_default(),
                            p.y.as_str(),
                            if p.percent { "%" } else { "" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        Ok(
            json!({"kind":self.kind,"title":self.title.presentation(),"horizontal":horizontal,"zero":zero,"y_ticks":y_ticks,"x_ticks":x_ticks,"copy_data":copy_data,
            "points":self.points.iter().enumerate().map(|(i,p)| json!({"label":p.label.presentation(),"x":xs[i],"y":ys[i],"value":format!("{}{}",p.y.as_str(),if p.percent {"%"} else {""}),"x_value":p.x.as_ref().map(Number::as_str),"share":shares[i],"percent":Number::new(shares[i]*100.0).and_then(|n|n.fixed(2)).unwrap_or_default()})).collect::<Vec<_>>()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Text, data::Point};
    #[test]
    fn chart_axes_remain_finite_at_numeric_extremes_and_pie_shares_are_normalized() {
        let plain = |s| Text::plain(s, Default::default()).unwrap();
        let mut chart = Chart {
            kind: ChartKind::Line,
            title: plain("Values"),
            points: vec![
                Point {
                    label: plain("Low"),
                    x: None,
                    y: Number::new(-f64::MAX).unwrap(),
                    percent: false,
                },
                Point {
                    label: plain("High"),
                    x: None,
                    y: Number::new(f64::MAX).unwrap(),
                    percent: false,
                },
            ],
        };
        let view = chart.presentation().unwrap();
        assert_eq!(view["points"][0]["y"], 0.0);
        assert_eq!(view["points"][1]["y"], 1.0);
        assert_eq!(view["zero"], 0.5);
        for tick in view["y_ticks"].as_array().unwrap() {
            assert!(tick.as_str().unwrap().parse::<f64>().unwrap().is_finite());
        }
        chart.kind = ChartKind::Pie;
        chart.points[0].y = Number::new(f64::MAX).unwrap();
        let view = chart.presentation().unwrap();
        assert_eq!(view["points"][0]["share"], 0.5);
        assert_eq!(view["points"][1]["percent"], "50");
        chart.points[0].label = crate::parse("spoiler::Hidden;", Default::default()).unwrap();
        assert!(chart.presentation().unwrap()["copy_data"].is_null());
        chart.kind = ChartKind::Bar;
        for p in &mut chart.points {
            p.y = Number::new(0.0).unwrap();
        }
        assert_eq!(chart.presentation().unwrap()["points"][0]["y"], 0.5);
    }
}
