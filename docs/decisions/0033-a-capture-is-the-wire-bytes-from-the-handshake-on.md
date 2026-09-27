# ADR 0033: A capture is the wire bytes from the handshake on

## Status

Accepted

## Context

The specification gives `record` and `replay` three sentences, SPEC 15:

> **`record` and `replay`.** `record` appends framed records to a file. `replay`
> feeds that file to a consumer with no DCS running. The format is the wire
> format, appended — no second format exists.

It does not say whether the two frames the broker sends before any record,
the handshake and the `AuthResult`, are part of the file, and it does not
say whether a frame is written as read or decoded and written again.

`dcsb` decodes the wire through its own `Envelope`, which declares only the
fields its verbs read: `Handshake` has four fields and the CLI's copy
declares one. A frame re-encoded through those types loses the rest.

`tail` prints a gap line before any frame whose `seq` skips past the one
before it, from zero. A file that starts at a record starts at `seq` 3 or
later, and every `tail` over it opens with a gap.

## Decision

`record` writes each frame's bytes as it read them, length prefix included,
from the first frame the connection carries. The handshake and the
`AuthResult` are in the file, and nothing is re-encoded.

Each frame is flushed as it is written, so a stopped run leaves whole
frames and nothing else. A second run onto the same file appends; the join
reads as `seq` going back to 1, which `tail` prints as out of order, and
that is what happened. A file that ends inside a frame is refused, with the
offset of its last whole frame, because a session appended after a partial
frame reads as garbage from the join on: the partial frame's length prefix
takes in the next session's first bytes.

Alternatives:

- **Decode and re-encode each frame.** Drops every field the CLI does not
  declare, and is the second format the specification says does not exist.
- **Start the file at the first record.** Every `tail` over it opens with a
  gap line, and a consumer replayed into never sees the handshake it would
  see live.
- **Truncate a partial frame and append.** Destroys bytes the user did not
  ask to lose, on a file that may be their only copy.

## Consequences

A capture is what a consumer would have read from a live bridge, so
`replay` needs no knowledge of the frames it serves, and a consumer under
test cannot tell the two apart by the stream alone.

A capture holds the token's answer for the token `record` used. A consumer
replayed into with a different token reads `ok=true` all the same; ADR 0034
holds that side.

A flush per frame is a system call per frame. Under a burst `record` can
become the slow consumer, and the bridge evicts behind it as it would behind
any other; the capture then holds the gaps `tail` would have printed. A
buffered writer would need a signal handler to leave whole frames on
Ctrl-C, and no crate in the workspace provides one; this reopens if a
captured burst shows gaps a live `tail` did not.
