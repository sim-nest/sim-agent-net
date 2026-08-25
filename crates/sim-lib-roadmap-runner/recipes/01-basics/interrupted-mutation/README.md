# Recover an interrupted mutation

Start `sim roadmap run` only with both `--disposable-checkout PATH` and
`--local-authority-token TOKEN`, then invoke `resume` with the same execution
and content identities. A generation change is refused rather than hot-swapped.
