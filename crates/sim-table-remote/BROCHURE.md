# sim-table-remote

In one line: The piece that makes a table living on another machine look and act like a local table in SIM.

## What it gives you

This crate lets SIM treat a table that actually lives somewhere else as if it were right at hand. It presents a remote table as an ordinary table object, so the code that reads and works with it does not need to know the data sits on another machine. Alongside the table it provides a small record that describes what the remote table is, and the adapter that connects the local view to the remote source behind it. The effect is that distance disappears from the caller's point of view: you work with the table through the same everyday moves you would use for one held locally, and the crossing of the gap is handled underneath.

## Why you will be glad

- You work with a remote table using the same familiar table moves as a local one.
- The code that uses the data stays simple, since the network trip is hidden away.
- A clear description travels with each remote table, so what it is and where it lives stays plain.

## Where it fits

This crate is the remote-data member of SIM's table family. It plays the same role a local table does, but sources its contents from another machine through an adapter. When part of your data belongs on a different node yet you want it to behave like any other table in the runtime, this is the piece that projects it into place.
