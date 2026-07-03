use sim_kernel::{CapabilityName, Error, Result};
use sim_lib_mcp::McpProfile;

/// Parsed command-line options for the MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOptions {
    /// Transport to serve on.
    pub transport: Transport,
    /// Visibility profile filtering the surface.
    pub profile: McpProfile,
    /// Capabilities granted to the session.
    pub capabilities: Vec<CapabilityName>,
    /// Whether to log diagnostics to stderr.
    pub log_stderr: bool,
}

/// Transport the MCP server listens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Line-delimited MCP over standard input and output.
    Stdio,
    /// MCP over HTTP (currently rejected by [`run`](crate::run)).
    Http {
        /// `host:port` address to bind.
        address: String,
        /// HTTP route path serving MCP.
        route: String,
    },
}

impl CliOptions {
    /// Parses options from the process arguments (skipping the program name).
    pub fn parse() -> Result<Self> {
        Self::parse_from(std::env::args().skip(1))
    }

    /// Parses options from an explicit argument iterator.
    pub fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut transport = None;
        let mut profile = McpProfile::all();
        let mut capabilities = Vec::new();
        let mut route = "/mcp".to_owned();
        let mut log_stderr = false;

        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--stdio" => set_transport(&mut transport, Transport::Stdio)?,
                "--http" => {
                    let address = next_arg(&mut iter, "--http expects host:port")?;
                    set_transport(
                        &mut transport,
                        Transport::Http {
                            address,
                            route: route.clone(),
                        },
                    )?;
                }
                "--route" => {
                    route = next_arg(&mut iter, "--route expects a path")?;
                    if let Some(Transport::Http {
                        route: http_route, ..
                    }) = &mut transport
                    {
                        *http_route = route.clone();
                    }
                }
                "--profile" => {
                    let name = next_arg(&mut iter, "--profile expects a name")?;
                    profile = parse_profile(&name)?;
                }
                "--allow-tool" => {
                    profile = profile.with_allowed_name(next_arg(
                        &mut iter,
                        "--allow-tool expects a name or glob",
                    )?);
                }
                "--deny-tool" => {
                    profile = profile.with_denied_name(next_arg(
                        &mut iter,
                        "--deny-tool expects a name or glob",
                    )?);
                }
                "--cap" => {
                    capabilities.push(CapabilityName::new(next_arg(
                        &mut iter,
                        "--cap expects a capability name",
                    )?));
                }
                "--no-default-tools" => {
                    profile = profile.with_denied_name("*");
                }
                "--log-stderr" => log_stderr = true,
                other => {
                    return Err(Error::Eval(format!(
                        "unknown sim-mcp-server option {other}"
                    )));
                }
            }
        }

        Ok(Self {
            transport: transport.unwrap_or(Transport::Stdio),
            profile,
            capabilities,
            log_stderr,
        })
    }
}

fn set_transport(slot: &mut Option<Transport>, transport: Transport) -> Result<()> {
    if slot.is_some() {
        return Err(Error::Eval(
            "sim-mcp-server accepts one transport option".to_owned(),
        ));
    }
    *slot = Some(transport);
    Ok(())
}

fn parse_profile(name: &str) -> Result<McpProfile> {
    match name {
        "default" => Ok(McpProfile::all()),
        other => Err(Error::Eval(format!("unknown MCP profile {other}"))),
    }
}

fn next_arg(iter: &mut impl Iterator<Item = String>, message: &'static str) -> Result<String> {
    iter.next().ok_or_else(|| Error::Eval(message.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stdio_profile_caps_and_filters() {
        let opts = CliOptions::parse_from([
            "--stdio".to_owned(),
            "--profile".to_owned(),
            "default".to_owned(),
            "--allow-tool".to_owned(),
            "core.*".to_owned(),
            "--deny-tool".to_owned(),
            "*.danger*".to_owned(),
            "--cap".to_owned(),
            "mcp.tools.call".to_owned(),
            "--log-stderr".to_owned(),
        ])
        .unwrap();

        assert_eq!(opts.transport, Transport::Stdio);
        assert_eq!(
            opts.capabilities,
            vec![CapabilityName::new("mcp.tools.call")]
        );
        assert!(opts.log_stderr);
        assert!(opts.profile.allows_name("core.echo"));
        assert!(!opts.profile.allows_name("core.dangerous"));
    }

    #[test]
    fn duplicate_transport_is_rejected() {
        let err =
            CliOptions::parse_from(["--stdio".to_owned(), "--http".to_owned(), "x:1".to_owned()])
                .unwrap_err();

        assert!(format!("{err}").contains("one transport"));
    }
}
