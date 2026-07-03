//! Standalone MCP server entry point for SIM.
//!
//! Parses [`CliOptions`], builds a runtime [`Cx`] with the MCP codec and
//! library installed, and serves the Model Context Protocol over the selected
//! [`Transport`] (currently stdio).

#![deny(missing_docs)]

mod cli;

use std::io::{BufRead, Write};
use std::sync::Arc;

use sim_codec_mcp::McpCodecLib;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Error, Result};
use sim_lib_mcp::stdio::{StdioOptions, mcp_stdio_capability, run_stdio};
use sim_lib_mcp::{McpRouter, McpSession, install_mcp_lib};

pub use cli::{CliOptions, Transport};

/// Parses options from process arguments and serves on the standard streams.
pub fn run_from_env() -> Result<()> {
    run(
        CliOptions::parse()?,
        std::io::stdin().lock(),
        std::io::stdout(),
        std::io::stderr(),
    )
}

/// Serves MCP per `opts` over the given reader, writer, and diagnostics streams.
///
/// Only the stdio transport is supported; selecting HTTP returns an error.
pub fn run<R, W, E>(opts: CliOptions, reader: R, writer: W, diagnostics: E) -> Result<()>
where
    R: BufRead,
    W: Write,
    E: Write,
{
    match &opts.transport {
        Transport::Stdio => run_stdio_transport(opts, reader, writer, diagnostics),
        Transport::Http { .. } => Err(Error::Eval(
            "sim-mcp-server --http is disabled; use --stdio".to_owned(),
        )),
    }
}

fn run_stdio_transport<R, W, E>(
    opts: CliOptions,
    reader: R,
    writer: W,
    diagnostics: E,
) -> Result<()>
where
    R: BufRead,
    W: Write,
    E: Write,
{
    let mut cx = runtime_cx()?;
    let mut session = McpSession::new("stdio", opts.profile);
    session = session.with_granted_capability(mcp_stdio_capability());
    for capability in opts.capabilities {
        session = session.with_granted_capability(capability);
    }
    let mut router = McpRouter::new(session);
    run_stdio(
        &mut cx,
        &mut router,
        reader,
        writer,
        diagnostics,
        StdioOptions {
            log_stderr: opts.log_stderr,
        },
    )?;
    Ok(())
}

fn runtime_cx() -> Result<Cx> {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let codec = McpCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&codec)?;
    install_mcp_lib(&mut cx)?;
    Ok(cx)
}

#[cfg(test)]
mod tests;
