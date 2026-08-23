This recipe keeps the command adapter narrow: it exposes the loadable `sim
provider` surface and redaction-safe rendering, while provider discovery,
credentials, probing, and opening remain owned by `sim-lib-provider`.
