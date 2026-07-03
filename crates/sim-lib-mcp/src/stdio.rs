use std::io::{BufRead, Write};

use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_mcp::{McpEnvelope, McpError, McpErrorEnvelope, PARSE_ERROR, envelope_to_expr};
use sim_kernel::{CapabilityName, Cx, EncodeOptions, Error, Expr, ReadPolicy, Result, Symbol};

use crate::McpRouter;

/// Options controlling the stdio MCP transport loop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StdioOptions {
    /// Whether parse errors are logged to the diagnostics stream.
    pub log_stderr: bool,
}

/// Tally of frames processed by a [`run_stdio`] loop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StdioSummary {
    /// Number of input lines read.
    pub frames_read: usize,
    /// Number of reply frames written.
    pub replies_written: usize,
    /// Number of lines that failed to decode.
    pub parse_errors: usize,
}

/// Returns the capability gating the stdio MCP transport.
pub fn mcp_stdio_capability() -> CapabilityName {
    CapabilityName::new("mcp.stdio")
}

/// Runs a line-delimited MCP request/response loop over `reader`/`writer`.
///
/// Each input line is decoded with the MCP codec, routed through `router`, and
/// its replies written to `writer`; decode failures are reported as MCP parse
/// errors and counted in the returned [`StdioSummary`].
pub fn run_stdio<R, W, E>(
    cx: &mut Cx,
    router: &mut McpRouter,
    reader: R,
    mut writer: W,
    mut diagnostics: E,
    options: StdioOptions,
) -> Result<StdioSummary>
where
    R: BufRead,
    W: Write,
    E: Write,
{
    let mut summary = StdioSummary::default();
    for line in reader.lines() {
        let line = line.map_err(io_error_to_host)?;
        summary.frames_read += 1;
        let replies = match decode_line(cx, line) {
            Ok(expr) => router.handle_exprs(cx, expr)?,
            Err(error) => {
                summary.parse_errors += 1;
                if options.log_stderr {
                    writeln!(diagnostics, "sim-mcp-server: parse error: {error}")
                        .map_err(io_error_to_host)?;
                }
                vec![parse_error_expr(error.to_string())]
            }
        };
        for reply in replies {
            write_reply(cx, &mut writer, &reply)?;
            summary.replies_written += 1;
        }
    }
    writer.flush().map_err(io_error_to_host)?;
    diagnostics.flush().map_err(io_error_to_host)?;
    Ok(summary)
}

fn decode_line(cx: &mut Cx, line: String) -> Result<Expr> {
    decode_with_codec(
        cx,
        &mcp_codec_symbol(),
        Input::Text(line),
        ReadPolicy::default(),
    )
}

fn write_reply(cx: &mut Cx, writer: &mut impl Write, expr: &Expr) -> Result<()> {
    let output = encode_with_codec(cx, &mcp_codec_symbol(), expr, EncodeOptions::default())?;
    writeln!(writer, "{}", output.into_text()?).map_err(io_error_to_host)
}

fn parse_error_expr(message: String) -> Expr {
    envelope_to_expr(&McpEnvelope::Error(McpErrorEnvelope {
        id: Expr::Nil,
        error: McpError {
            code: PARSE_ERROR,
            message: "parse error".to_owned(),
            data: Expr::String(message),
        },
    }))
}

fn mcp_codec_symbol() -> Symbol {
    Symbol::qualified("codec", "mcp")
}

fn io_error_to_host(error: std::io::Error) -> Error {
    Error::HostError(error.to_string())
}
