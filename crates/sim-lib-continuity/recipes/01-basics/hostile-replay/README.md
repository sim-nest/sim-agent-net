# Hostile continuity replay

The trace accepts an observation and cancellation while proving duplicates are
idempotent and reordered and post-cancel events are refused. It then deletes
the derived state, rebuilds from the fenced journal, and compares the result.

Run with `cargo run -p sim-lib-continuity-recipe-hostile-replay`.
