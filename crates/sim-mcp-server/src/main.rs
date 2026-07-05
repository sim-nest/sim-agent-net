//! The `sim-mcp-server` binary: a thin bootloader that serves MCP over stdio.
//!
//! `sim-mcp-server ARGS...` boots through [`sim_run_core::Bootloader`] (via
//! [`sim_lib_mcp::mcp_bootloader`]) -- the same runtime the `sim` binary uses -- with
//! the MCP codec and serve library loaded, and dispatches the `cli/main/mcp`
//! entrypoint. It is equivalent to `sim mcp ARGS...`. This binary constructs no `Cx`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::ffi::OsString;
use std::process;

fn main() {
    // parse_args drops argv[0]; set the mcp boot codec and inject the `mcp` verb so
    // `sim-mcp-server --stdio` dispatches the serve library like `sim mcp --stdio`.
    let mut args: Vec<OsString> = ["sim-mcp-server", "--codec", "mcp", "mcp"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.extend(std::env::args_os().skip(1));

    let code = sim_lib_mcp::mcp_bootloader()
        .run(args)
        .unwrap_or_else(|error| {
            eprintln!("sim-mcp-server: {error}");
            2
        });
    process::exit(code);
}
