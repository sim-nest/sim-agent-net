# Process JSON-stdio runner (descriptor)

Documents the process agent runner speaking JSON over stdio to a subprocess, driven here by a
fixture. Spawning and talking to a process is I/O outside the cookbook sandbox eval stack, so the
exchange is documented rather than run.
