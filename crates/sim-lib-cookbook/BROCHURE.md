# sim-lib-cookbook

In one line: A built-in collection of worked recipes you can browse, search, and actually run inside SIM.

## What it gives you

This crate turns SIM's recipe collection into live commands you can use while the system is running. You can list the books and chapters, look up a specific recipe, search across them, and step to the next one in a sequence. Recipes are more than reading material: a recipe can be run, and when it runs SIM reads its setup, carries it out, and checks that the results match what the recipe promised. That makes each recipe a small, self-checking example rather than a snippet you copy by hand and hope works. The same shared collection feeds the command line, the web view, and the help and browse surfaces, so everyone sees one consistent set of examples.

## Why you will be glad

- You can find a worked example by searching, then run it on the spot to see it in action.
- Each recipe checks its own results, so an example that has quietly stopped working is caught.
- The command line, web, and help all draw from one recipe collection, so guidance stays consistent.

## Where it fits

This crate is the bridge between SIM's kernel-free recipe engine and the running system. The recipes and the engine live elsewhere, kept independent; this piece registers the commands that let a live SIM browse and run them. It gives every surface a single path to the same examples instead of each one growing its own copy.
