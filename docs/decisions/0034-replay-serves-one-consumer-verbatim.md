# ADR 0034: `replay` serves one consumer verbatim, authenticates nobody and keeps no time

## Status

Accepted

## Context

The specification says what `replay` is for and nothing about how it
behaves as a peer, SPEC 15:

> `replay` feeds that file to a consumer with no DCS running.

A consumer connects to a bridge and sends an `Auth` before it reads
anything. With no DCS running there is no bridge and no token table, so
nothing can check what the consumer sends. The capture holds the answer the
bridge gave the token `record` used (ADR 0033).

A capture carries no wall clock. A frame carries a mission time only inside
an epoch, and the frames before the first `EpochOpened` carry none, so no
rate can be read from the file for the whole of it.

Task 6.8's completion condition is that a consumer rejects a stale frame
`dcsb replay` served it; task 9.8's is a load test at ten times a captured
stream's rate, driven by `dcsb mock` or `dcsb replay`.

## Decision

`replay` listens, accepts one consumer, writes the capture to it frame for
frame, closes, and exits. It reads and discards everything the consumer
sends, from the accept to the close, so the recorded `AuthResult` is what
answers the consumer's `Auth`, whatever token it carries. It writes frames
as fast as the consumer reads them.

It listens on its own address argument, `127.0.0.1:7743` by default, beside
the bridge's, so a bridge and a replay can run on one machine. Port 0 takes
a free one and the `listening on` line names it, before the accept, so a
script can start the consumer from that line.

The close is a write-side shutdown followed by a read to the consumer's end
of stream. A close over unread inbound bytes is answered with a reset on
Linux, and a reset can take the frames still in the consumer's receive
buffer with it, so the consumer's last frames would vanish there and
nowhere else.

Alternatives:

- **Check the token against a file.** Builds a second token table, and the
  capture's `AuthResult` answers whatever it decided anyway.
- **Serve every consumer that connects, until stopped.** Nothing in the
  build asks for it, and a process that never exits is one a script must
  kill.
- **Pace frames by their mission times.** Absent outside an epoch, so the
  pace would be undefined for the frames before the first boundary and for
  every capture of a bridge between missions.

## Consequences

A consumer under test reads a stream a live bridge could have sent, at a
rate no bridge would send it. A consumer that depends on arrival timing is
not tested by `replay` as it stands; a `--pace` flag can be added with the
format unchanged, and a captured stream's rate for 9.8 is measured from the
mission times inside an epoch.

`replay` feeds a consumer and loads no broker. Task 9.8's load on the broker
rests on `mock`, task 2.C6, whose completion condition names 9.8's load
generator; `replay` there drives the consumer side.

Task 6.8's stale-frame check has what it needs: a capture from one mission
served into a consumer that has since seen a later epoch.

The bind address is a second well-known port beside the bridge's, and
`doctor` does not know it; a `replay` left running holds 7743 and the next
one exits 2 naming the address.
