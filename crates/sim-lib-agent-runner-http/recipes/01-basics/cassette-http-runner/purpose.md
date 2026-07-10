# HTTP cassette runner (descriptor)

Documents the HTTP agent runner replaying a recorded request/response cassette (a POST exchange).
The runner performs network I/O, which the cookbook sandbox eval stack does not execute, so the
cassette-replay flow is documented rather than run.
