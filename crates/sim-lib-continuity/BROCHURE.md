# Continuity without hidden authority

`sim-lib-continuity` turns an intermittent multi-role session into durable,
replayable data. Plans declare services, fallbacks, freshness, retention,
disclosure, and network limits. A pure reducer then emits intents for a host to
interpret; candidate routes never become capabilities. Fenced journal appends
make restart and hostile replay deterministic without introducing another log.
