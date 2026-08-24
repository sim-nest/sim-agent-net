# sim-lib-roadmap

`sim-lib-roadmap` gives the pure values owned by `sim-source-deck`,
`sim-roadmap-core`, `sim-roadmap-refine`, and `sim-roadmap-plan` ordinary SIM
faces. It supplies bounded tagged `Expr` projections, strict inverses, Cards,
Shapes, and read constructors. Its strict v3 adapter composes the document and
config codecs: it interprets their decoded structures, content-pins only bytes
from caller-owned maps, and keeps imported lifecycle observations outside the
native roadmap identity. It contains no line or TOML parser, scanner, journal,
filesystem access, model access, process launch, or outward operation.

`V3Importer` turns top-level legacy phases into children of one native root,
typed dependencies into edges, tasks into checkpoints, and prose or fenced
snippets into visibly ungrounded guides. `render_native` is the stable recursive
form. `render_v3` is intentionally narrower and returns every precise loss path
when native structure cannot be represented by flat v3 Markdown.

The projection is an open record: optional extension fields are retained when
their names begin with `x-`; unknown structural fields and unknown value kinds
are rejected. Every admitted value carries a semantic identifier over its
kind and body, so a forged identifier or altered grounding assertion fails at
the same admission boundary regardless of whether it arrived directly or by
read construction.
