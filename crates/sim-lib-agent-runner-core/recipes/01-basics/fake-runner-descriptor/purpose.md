# Fake runner descriptor

Spawning a runner needs a host process, which the sandbox has no capability for. This recipe is a **descriptor** (tagged `sandbox-descriptor`): it shows the real
surface shape rather than a live result, because that result cannot be reproduced in
the cookbook's read-eval sandbox.
