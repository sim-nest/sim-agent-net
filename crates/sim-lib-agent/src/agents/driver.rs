use super::model::{agent_from_value, resolve_agent_address};
use crate::installed_codecs;
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{Connection, LineDriver, ServerAddress};

const AGENT_DRIVE_CAPABILITY: &str = "agent-drive";

pub(crate) fn agent_line_driver_factory(
    cx: &mut Cx,
    spec: &Expr,
) -> Result<Option<Box<dyn LineDriver>>> {
    cx.require(&CapabilityName::new(AGENT_DRIVE_CAPABILITY))?;
    let target = driver_agent_target(spec)?;
    if let Ok(value) = cx.eval_expr(target.clone())
        && let Ok(agent) = agent_from_value(&value)
    {
        let state = agent
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("agent state"))?;
        let default_codec = state.default_codec.clone();
        let supported_codecs = state.supported_codecs.clone();
        drop(state);
        let connection = Connection::with_session(
            ServerAddress::Local,
            default_codec,
            supported_codecs,
            agent.site()?,
            None,
            agent.policy.clone(),
        )?;
        return Ok(Some(Box::new(AgentLineDriver {
            connection,
            output: String::new(),
        })));
    }
    let name = match target {
        Expr::Symbol(symbol) => symbol,
        _ => {
            return Err(Error::Eval(
                "agent driver expects (agent <symbol>)".to_owned(),
            ));
        }
    };
    let address = ServerAddress::Agent {
        agent: name.to_string(),
    };
    let Some(resolved) = resolve_agent_address(cx, &address, &installed_codecs(cx))? else {
        return Err(Error::Eval(format!(
            "agent driver target {} is not started",
            name
        )));
    };
    let connection = Connection::with_session(
        address,
        resolved.selected_codec,
        resolved.supported_codecs,
        resolved.site,
        None,
        sim_lib_server::IsolationPolicy::default(),
    )?;
    Ok(Some(Box::new(AgentLineDriver {
        connection,
        output: String::new(),
    })))
}

struct AgentLineDriver {
    connection: Connection,
    output: String,
}

impl LineDriver for AgentLineDriver {
    fn read_line(&mut self, cx: &mut Cx, prompt: &str) -> Result<Option<String>> {
        let request = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("driver")),
                Expr::Symbol(Symbol::new("agent")),
            ),
            (
                Expr::Symbol(Symbol::new("prompt")),
                Expr::String(prompt.to_owned()),
            ),
            (
                Expr::Symbol(Symbol::new("output")),
                Expr::String(self.output.clone()),
            ),
        ]);
        let reply = self.connection.request(cx, request, None, Vec::new())?;
        match reply.object().as_expr(cx)? {
            Expr::Nil => Ok(None),
            Expr::String(text) => Ok(Some(text)),
            Expr::Map(entries) => {
                if entries.iter().any(|(key, value)| {
                    matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "done")
                        && matches!(value, Expr::Bool(true))
                }) {
                    return Ok(None);
                }
                let line = entries.iter().find_map(|(key, value)| match (key, value) {
                    (Expr::Symbol(symbol), Expr::String(text))
                        if symbol.name.as_ref() == "line" =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                });
                Ok(line)
            }
            other => Err(Error::Eval(format!(
                "agent driver expected nil, string, or {{line ...}}, found {other:?}"
            ))),
        }
    }

    fn write_output(&mut self, _cx: &mut Cx, output: &str) -> Result<()> {
        self.output.push_str(output);
        Ok(())
    }
}

fn driver_agent_target(spec: &Expr) -> Result<Expr> {
    match spec {
        Expr::List(items) | Expr::Vector(items) => match items.get(1) {
            Some(expr) => Ok(expr.clone()),
            _ => Err(Error::Eval(
                "agent driver expects (agent <symbol>)".to_owned(),
            )),
        },
        Expr::Symbol(symbol) if symbol.name.as_ref() == "agent" => Err(Error::Eval(
            "agent driver requires an agent name".to_owned(),
        )),
        _ => Err(Error::Eval(
            "agent driver expects (agent <symbol>)".to_owned(),
        )),
    }
}
