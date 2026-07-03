use std::process;

fn main() {
    if let Err(error) = sim_mcp_server::run_from_env() {
        eprintln!("sim-mcp-server: {error}");
        process::exit(1);
    }
}
