# Local model placement

This recipe records the local model path as a placement graph. A runner is
placed under `model-site:local`, the prompt graph realizes through
`model/cached` and `model/at`, and the fixture answer is deterministic.

The fixture uses `runner/fake` so cookbook validation stays offline. The same
shape accepts the loadable local backend when an operator grants the local model
capability and loads the site.
