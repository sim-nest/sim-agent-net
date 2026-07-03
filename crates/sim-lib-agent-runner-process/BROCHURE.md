# sim-lib-agent-runner-process

In one line: The connector that lets SIM use a model or tool that runs as a separate program on your computer.

## What it gives you

This crate lets SIM drive a model by launching a local program and talking to it through its input and output. SIM sends the request in, the program does its work, and SIM reads the result back. It supports both a plain text conversation and a structured exchange where the request and reply are passed as JSON, and it can read the program's output line by line so streamed answers arrive as they are produced. This is the natural way to wire in a command-line model tool, a script, or any local helper that reads a request and prints a reply, without that tool needing to know anything about SIM.

## Why you will be glad

- Any local command-line model or script becomes usable from SIM without changing that tool.
- You can keep a model in its own program, isolated from the main runtime, and still call it cleanly.
- Streamed, line-by-line output means long answers show up as they come rather than all at once.

## Where it fits

This is the subprocess member of SIM's model runner family. It follows the same shared runner contract as the web and in-process runners, so an agent calls a local program exactly the way it calls any other model. Use it when the model or helper you want already exists as a separate executable and you would rather run it than embed it.
