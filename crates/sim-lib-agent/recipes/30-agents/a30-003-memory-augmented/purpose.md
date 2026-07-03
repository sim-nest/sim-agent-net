# Memory Augmented Answer

This recipe records a deterministic answer that consults working memory and
episodic memory. The setup quotes the current goal, table-backed episodes, the
labeled retrieve step, the recalled episode ids, and the answer derived from
those episodes.

The fixture stays local and synthetic. Its retrieve step is a table lookup so
recipe execution remains deterministic while still showing the memory shape an
agent uses for recall.
