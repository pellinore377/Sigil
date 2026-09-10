use crate::{
    Error,
    data::{Data, Diagram, DiagramKind},
};
use serde_json::{Value, json};
use std::collections::VecDeque;

impl Diagram {
    pub fn presentation(&self) -> Result<Value, Error> {
        Data::Diagram(self.clone()).validate(Default::default())?;
        if self.kind == DiagramKind::Timeline {
            return Ok(
                json!({"kind":self.kind,"title":self.title.presentation(),"width":440,"height":self.entries.len()*112+24,
                "nodes":[],"edges":[],"entries":self.entries.iter().map(|(date,label)|json!({"date":date.presentation(),"label":label.presentation()})).collect::<Vec<_>>()}),
            );
        }
        let count = self.nodes.len();
        let mut children = vec![Vec::new(); count];
        let mut parents = vec![0; count];
        for edge in &self.edges {
            children[edge.from as usize].push(edge.to as usize);
            parents[edge.to as usize] += 1;
        }
        let mut depth = vec![None; count];
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        for (index, &n) in parents.iter().enumerate() {
            if n == 0 {
                depth[index] = Some(0);
                queue.push_back(index);
            }
        }
        loop {
            if queue.is_empty() {
                let Some(index) = depth.iter().position(Option::is_none) else {
                    break;
                };
                depth[index] = Some(0);
                queue.push_back(index);
            }
            while let Some(node) = queue.pop_front() {
                order.push(node);
                for &child in &children[node] {
                    if depth[child].is_none() {
                        depth[child] = Some(depth[node].unwrap() + 1);
                        queue.push_back(child);
                    }
                }
            }
        }
        let mut position = vec![(0.0f64, 0.0f64); count];
        if self.kind == DiagramKind::Sequence {
            for (i, p) in position.iter_mut().enumerate() {
                *p = (24.0 + i as f64 * 208.0, 24.0);
            }
        } else if self.kind == DiagramKind::Mindmap {
            let root = parents.iter().position(|&n| n == 0).ok_or(Error::Invalid)?;
            let mut weights = vec![1usize; count];
            for &node in order.iter().rev() {
                if !children[node].is_empty() {
                    weights[node] = children[node].iter().map(|&i| weights[i]).sum();
                }
            }
            let mut sectors = vec![(0.0f64, std::f64::consts::TAU); count];
            for &node in &order {
                let (start, end) = sectors[node];
                let angle = (start + end) / 2.0;
                if node != root {
                    let radius = depth[node].unwrap() as f64
                        * (weights[root] as f64 * 180.0 / std::f64::consts::TAU).max(240.0);
                    position[node] = (angle.cos() * radius, angle.sin() * radius);
                }
                let mut at = start;
                for &child in &children[node] {
                    let end = at + (end - start) * weights[child] as f64 / weights[node] as f64;
                    sectors[child] = (at, end);
                    at = end;
                }
            }
            let min_x = position.iter().map(|p| p.0).fold(0.0, f64::min);
            let min_y = position.iter().map(|p| p.1).fold(0.0, f64::min);
            for p in &mut position {
                p.0 += 24.0 - min_x;
                p.1 += 24.0 - min_y;
            }
        } else {
            let mut layers =
                vec![Vec::new(); depth.iter().flatten().copied().max().unwrap_or(0) + 1];
            for &node in &order {
                layers[depth[node].unwrap()].push(node);
            }
            let width = layers.iter().map(Vec::len).max().unwrap_or(1) as f64 * 208.0;
            for (level, layer) in layers.iter().enumerate() {
                let start = (width - layer.len() as f64 * 208.0) / 2.0 + 24.0;
                for (i, &node) in layer.iter().enumerate() {
                    position[node] = (start + i as f64 * 208.0, 24.0 + level as f64 * 144.0);
                }
            }
        }
        let width = position.iter().map(|p| p.0 + 184.0).fold(0.0, f64::max)
            + if matches!(self.kind, DiagramKind::Flow | DiagramKind::State)
                && self
                    .edges
                    .iter()
                    .any(|e| position[e.to as usize].1 <= position[e.from as usize].1)
            {
                if self.edges.iter().any(|e| {
                    position[e.to as usize].1 <= position[e.from as usize].1
                        && !e.label.body().is_empty()
                }) {
                    168.0
                } else {
                    72.0
                }
            } else {
                0.0
            };
        let height = if self.kind == DiagramKind::Sequence {
            144.0 + self.edges.len() as f64 * 96.0
        } else {
            position.iter().map(|p| p.1 + 96.0).fold(0.0, f64::max)
        };
        Ok(
            json!({"kind":self.kind,"title":self.title.presentation(),"width":width,"height":height,"entries":[],
            "nodes":self.nodes.iter().enumerate().map(|(i,n)|json!({"label":n.label.presentation(),"shape":n.shape,"x":position[i].0,"y":position[i].1})).collect::<Vec<_>>(),
            "edges":self.edges.iter().enumerate().map(|(i,e)|json!({"from":e.from,"to":e.to,"label":e.label.presentation(),"dashed":e.dashed,"y":144+i*96})).collect::<Vec<_>>()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Origin, Parsed};
    #[test]
    fn diagram_layouts_preserve_topology_and_bound_cycles_and_large_hierarchies() {
        let origin = Origin {
            message: [1; 32],
            creator: [2; 32],
            created_at: 1_767_225_600,
            timezone: Some("UTC"),
        };
        for source in [
            "diagram::flow::Flow\n- Start -> Work\n- Work -> Start;",
            "diagram::state::Call\n- Idle -> Ringing [invite]\n- Ringing -> Idle [decline];",
            "diagram::sequence::Delivery\n- Client -> Server: send\n- Server --> Client: reply;",
            "diagram::org::Team\n- Lead -> A\n- Lead -> B\n- B -> C;",
            "diagram::mindmap::Ideas\n- Root -> A\n- Root -> B\n- A -> C;",
            "diagram::timeline::Project\n- March = Start\n- September = Finish;",
        ] {
            let Parsed::Card(card) = crate::parse_card(source, origin, Default::default())
                .unwrap()
                .content
            else {
                panic!("{source}")
            };
            let crate::structured::Construct::Data(Data::Diagram(diagram)) = card.content else {
                panic!()
            };
            let original = serde_json::to_vec(&diagram).unwrap();
            let view = diagram.presentation().unwrap();
            assert_eq!(view["edges"].as_array().unwrap().len(), diagram.edges.len());
            for node in view["nodes"].as_array().unwrap() {
                assert!(node["x"].as_f64().unwrap() >= 0.0);
                assert!(node["y"].as_f64().unwrap() >= 0.0);
            }
            assert_eq!(serde_json::to_vec(&diagram).unwrap(), original);
            assert!(view["width"].as_f64().unwrap() > 0.0);
            assert!(view["height"].as_f64().unwrap() > 0.0);
        }
        let rows = (0..255)
            .map(|i| format!("- N{i} -> N{}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let Parsed::Card(card) = crate::parse_card(
            &format!("diagram::org::Long\n{rows};"),
            origin,
            Default::default(),
        )
        .unwrap()
        .content
        else {
            panic!()
        };
        let crate::structured::Construct::Data(Data::Diagram(diagram)) = card.content else {
            panic!()
        };
        let view = diagram.presentation().unwrap();
        assert_eq!(view["nodes"].as_array().unwrap().len(), 256);
        assert!(view["height"].as_f64().unwrap() < 40_000.0);
    }
}
