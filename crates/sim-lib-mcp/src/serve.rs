//! The loadable MCP serve entrypoint.
//!
//! `McpServeLib` is a host-registered library exporting a `cli/main/mcp`
//! entrypoint. When the `sim` bootloader (or the `sim-mcp-server` shim, which is a
//! thin `sim_run_core::Bootloader`) dispatches the `mcp` verb, the entrypoint stands
//! up the MCP runtime IN THE BOOTLOADER-PROVIDED `Cx` (no `Cx::new`), parses the CLI
//! options from the boot envelope, and runs the stdio transport loop. The transport
//! (`run_stdio`), routing (`McpRouter`), and session logic live in this crate; this
//! module adds the bootloader handoff for the `mcp` serve verb.
//!
//! [`configure_mcp_bootloader`] registers the MCP codec + `mcp` verb onto an existing
//! [`Bootloader`], so a downstream composer (e.g. the batteries-included `sim`) can
//! stack MCP and other serve libraries onto one bootloader.

use std::io;
use std::sync::Arc;

use sim_codec_mcp::McpCodecLib;
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, CodecId, Cx, Error, Export, Expr, Lib, LibManifest,
    LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_run_core::{Bootloader, cli_main_entrypoint_symbol};

/// Registers the MCP boot codec (`codec/mcp`) and the `mcp` serve verb onto an
/// existing [`Bootloader`], returning it for further composition. A downstream binary
/// can stack this with other serve libraries onto one bootloader.
pub fn configure_mcp_bootloader(loader: Bootloader) -> Bootloader {
    loader
        .host_lib("codec/mcp", || Box::new(McpCodecLib::new(CodecId(1))))
        .host_verb(MCP_SERVE_VERB, "lib/mcp-serve", || {
            Box::new(McpServeLib::new())
        })
}

/// A standalone [`Bootloader`] pre-configured to serve MCP: the `codec/mcp` boot codec
/// plus the `mcp` serve verb. A thin `sim-mcp-server` binary is just
/// `mcp_bootloader().run(..)`.
pub fn mcp_bootloader() -> Bootloader {
    configure_mcp_bootloader(Bootloader::standard())
}

use crate::stdio::{StdioOptions, mcp_stdio_capability, run_stdio};
use crate::{McpProfile, McpRouter, McpSession, install_mcp_lib};

/// The verb the bootloader dispatches to this library (`sim mcp ...`).
pub const MCP_SERVE_VERB: &str = "mcp";

/// Returns the function symbol exported for the bootloader handoff.
pub fn mcp_serve_entrypoint_symbol() -> Symbol {
    cli_main_entrypoint_symbol(MCP_SERVE_VERB)
}

/// Loadable MCP serve library. Host-register it under a name and make it the default
/// source for the `mcp` verb so `sim mcp --stdio` (or `sim-mcp-server --stdio`)
/// dispatches its `cli/main/mcp` entrypoint, which runs the stdio transport loop.
#[derive(Clone, Debug, Default)]
pub struct McpServeLib;

impl McpServeLib {
    /// Creates an MCP serve library instance.
    pub fn new() -> Self {
        Self
    }
}

impl Lib for McpServeLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "mcp-serve"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: mcp_serve_entrypoint_symbol(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        let entrypoint = cx.factory().opaque(Arc::new(McpServeEntrypoint))?;
        linker.function_value(mcp_serve_entrypoint_symbol(), entrypoint)?;
        Ok(())
    }
}

#[derive(Clone)]
struct McpServeEntrypoint;

impl Object for McpServeEntrypoint {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("cli/main/mcp".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for McpServeEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for McpServeEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let opts = options_from_envelope(cx, args.values().first())?;
        match &opts.transport {
            Transport::Stdio => {}
            Transport::Http { .. } => {
                return Err(Error::Eval(
                    "sim-mcp-server --http is disabled; use --stdio".to_owned(),
                ));
            }
        }

        // Stand up the MCP runtime in the bootloader-provided cx. No Cx::new here.
        // The bootloader already loaded the `codec/mcp` boot codec; install the MCP
        // method surface so the router can dispatch protocol calls.
        install_mcp_lib(cx)?;

        let mut session =
            McpSession::new("stdio", opts.profile).with_granted_capability(mcp_stdio_capability());
        for capability in opts.capabilities {
            session = session.with_granted_capability(capability);
        }
        let mut router = McpRouter::new(session);
        run_stdio(
            cx,
            &mut router,
            io::stdin().lock(),
            io::stdout(),
            io::stderr(),
            StdioOptions {
                log_stderr: opts.log_stderr,
            },
        )?;
        cx.factory().bool(true)
    }
}

/// Parses [`CliOptions`] from the boot envelope's `args` field (skipping the verb).
fn options_from_envelope(cx: &mut Cx, envelope: Option<&Value>) -> Result<CliOptions> {
    let Some(envelope) = envelope else {
        return CliOptions::parse_from(std::iter::empty());
    };
    let args = envelope_args(cx, envelope)?;
    // args[0] is the verb ("mcp"), analogous to argv[0]; skip it.
    CliOptions::parse_from(args.into_iter().skip(1))
}

fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    let value = table.get(cx, Symbol::new("args"))?;
    let Some(list) = value.object().as_list() else {
        return Err(Error::Eval(
            "CLI envelope field args is not a list".to_owned(),
        ));
    };
    list.to_vec(cx, Some(64))?
        .into_iter()
        .map(|value| match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            other => Err(Error::Eval(format!(
                "CLI payload argument is not a string: {other:?}"
            ))),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// CLI options (relocated from the sim-mcp-server binary so the parser ships with
// the serve library the bootloader loads).
// ---------------------------------------------------------------------------

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
    /// MCP over HTTP (currently rejected by the serve entrypoint).
    Http {
        /// `host:port` address to bind.
        address: String,
        /// HTTP route path serving MCP.
        route: String,
    },
}

impl CliOptions {
    /// Parses options from an explicit argument iterator (flags only, no verb/program).
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
    fn serve_lib_exports_cli_main_mcp() {
        let manifest = McpServeLib::new().manifest();
        assert!(manifest.exports.iter().any(|export| matches!(
            export,
            Export::Function { symbol, .. } if symbol == &mcp_serve_entrypoint_symbol()
        )));
    }

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
