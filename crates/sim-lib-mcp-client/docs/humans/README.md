# Human guide

Implement `BindingPeer` over the existing HTTP or stdio binding. Supply an
`EndpointIdentity` with no credentials, then construct `Client` with explicit
policy, cache, input broker, and ledger. Call `import_cards` to obtain the one
canonical `SkillCard` projection and invoke its `McpCallable` through `Client`.
