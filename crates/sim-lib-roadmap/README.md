# sim-lib-roadmap

`sim-lib-roadmap` gives the pure values owned by `sim-source-deck`,
`sim-roadmap-core`, `sim-roadmap-refine`, and `sim-roadmap-plan` ordinary SIM
faces. It supplies bounded tagged `Expr` projections, strict inverses, Cards,
Shapes, and read constructors. It deliberately contains no document parser,
scanner, journal, filesystem access, model access, process launch, or outward
operation.

The projection is an open record: optional extension fields are retained when
their names begin with `x-`; unknown structural fields and unknown value kinds
are rejected. Every admitted value carries a semantic identifier over its
kind and body, so a forged identifier or altered grounding assertion fails at
the same admission boundary regardless of whether it arrived directly or by
read construction.
