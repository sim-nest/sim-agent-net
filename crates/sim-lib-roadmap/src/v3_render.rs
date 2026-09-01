use crate::*;
pub fn render_v3(roadmap: &NativeRoadmap) -> Result<String, LossReport> {
    let mut losses = Vec::new();
    if roadmap.root.children.iter().any(|p| !p.children.is_empty()) {
        losses.push(RenderLoss {
            path: "root.phases.children".into(),
            reason: "v3 phases are flat".into(),
        });
    }
    for (i, p) in roadmap.root.children.iter().enumerate() {
        for (j, g) in p.guides.iter().enumerate() {
            if g.grounded {
                losses.push(RenderLoss {
                    path: format!("root.phases[{i}].guides[{j}].grounded"),
                    reason: "v3 cannot retain native grounding".into(),
                });
            }
        }
    }
    if !losses.is_empty() {
        return Err(LossReport { losses });
    }
    let mut out = format!("# {}\n\n", roadmap.title);
    for p in &roadmap.root.children {
        out.push_str(&format!("### [ ] {} - {}\n", p.id, p.title));
        for d in &p.dependencies {
            out.push_str(&format!(
                "{}: {}\n",
                match d.kind {
                    EdgeKind::Requires => "DEPENDS_ON",
                    EdgeKind::After => "AFTER",
                },
                d.target
            ));
        }
        for c in &p.checkpoints {
            out.push_str(&format!("- [ ] {}\n", c.text));
        }
        for g in &p.guides {
            if let Some(l) = &g.language {
                out.push_str(&format!("```{l}\n{}\n```\n", g.text));
            } else {
                out.push_str(&format!("{}\n", g.text));
            }
        }
        out.push('\n');
    }
    Ok(out)
}
