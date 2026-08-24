use crate::*;
pub fn render_native(roadmap: &NativeRoadmap) -> String {
    let mut out = format!("# {}\n\nLimitations:\n", roadmap.title);
    for x in &roadmap.limitations {
        out.push_str(&format!("- {x}\n"));
    }
    render_phase(&roadmap.root, 2, &mut out);
    out
}
fn render_phase(p: &NativePhase, level: usize, out: &mut String) {
    out.push_str(&format!("\n{} {} - {}\n", "#".repeat(level), p.id, p.title));
    for d in &p.dependencies {
        out.push_str(&format!("- dependency/{:?}: {}\n", d.kind, d.target));
    }
    for o in &p.origins {
        out.push_str(&format!(
            "- origin/{:?}: {}{} @ {}\n",
            o.relation,
            o.path,
            o.fragment
                .as_ref()
                .map(|f| format!("#{f}"))
                .unwrap_or_default(),
            o.content_id
        ));
    }
    for c in &p.checkpoints {
        out.push_str(&format!("- [ ] {}\n", c.text));
    }
    for g in &p.guides {
        out.push_str("\n> UNGROUNDED LEGACY GUIDE");
        if let Some(l) = &g.language {
            out.push_str(&format!(" ({l})"));
        }
        out.push_str("\n> ");
        out.push_str(&g.text.replace('\n', "\n> "));
        out.push('\n');
    }
    for c in &p.children {
        render_phase(c, level + 1, out)
    }
}
