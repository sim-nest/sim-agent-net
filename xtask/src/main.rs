#![forbid(unsafe_code)]

mod file_sizes;
mod recipe_assertions;
#[cfg(test)]
mod recipe_assertions_tests;
mod recipe_coverage;
mod simdoc;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

// Run the agent-net recipe-corpus assertions before the shared simdoc task. The
// assertions are agent-net specific, so they live here rather than in the
// `simdoc` module, which is kept byte-identical to the canonical sim-tooling copy
// (see `simctl sync-xtask`).
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let program = args.first().map(String::as_str).unwrap_or("xtask");
    match args.get(1).map(String::as_str) {
        Some("simdoc") => {
            let root = std::env::current_dir().map_err(|err| format!("current dir: {err}"))?;
            let summary = recipe_assertions::check_repo(&root)?;
            println!(
                "simdoc: recipe assertions checked {} recipe(s), including {} 30-agents recipe(s); {} publishable package(s) have recipe coverage",
                summary.checked_recipes, summary.agent30_recipes, summary.publishable_packages
            );
            simdoc::run(args)
        }
        Some("check-file-sizes") => file_sizes::run(&args),
        _ => Err(format!(
            "usage: {program} simdoc [--check] | check-file-sizes"
        )),
    }
}
