use sim_kernel::{Expr, Symbol};
use sim_lib_agent_conduct_core::{AgentEvent, AgentJournal, AgentRunFrame, AgentUsage, symbols};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame = AgentRunFrame::standard(
        Symbol::qualified("run", "recipe"),
        Expr::String("inspect evidence".into()),
    );
    let mut journal = AgentJournal::new("graph:recipe", "bindings:recipe");
    journal.append(
        AgentEvent::new(symbols::event::STEP_COMPLETED(), Expr::Nil),
        frame.clone(),
        AgentUsage::default(),
        vec![],
        Expr::Nil,
    )?;
    journal.append(
        AgentEvent::new(symbols::event::STEP_COMPLETED(), Expr::Nil),
        frame,
        AgentUsage::default(),
        vec![],
        Expr::Nil,
    )?;
    journal.verify()?;
    println!("verified journal records: {}", journal.records().len());
    Ok(())
}
