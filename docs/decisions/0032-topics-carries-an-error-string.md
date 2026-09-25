# ADR 0032: `Topics` carries an error string

## Status

Accepted

## Context

The specification requires `GetTopics` to answer with an error before the
class table is registered, SPEC 5.2:

> **`GetTopics` answers with an error until the class table is registered.**
> The listener opens at the first `configure`, before the hook driver registers
> its tables (Section 5.1), so a window exists in which the registered set is
> empty. An empty `Topics` in that window is indistinguishable from a correct
> answer, which is the failure `GetSchema` already avoids by erroring until the
> schema hand-off completes. `GetTopics` does the same.

Its `Topics` message has one field, SPEC 5.2:

```proto
message Topics {
  repeated TopicEntry topic = 1;
}
```

so the message the specification gives cannot carry the error the
specification requires. `Schema`, which it names as the precedent, has the
field: `optional string error = 2`, set instead of the descriptor set, with
exactly one of the two present.

## Decision

`Topics` carries `optional string error = 2`, as `Schema` does, and exactly
one of the list and the error is set. Before a class table is registered
the answer is the error and no entries; after, it is the entries the token
covers and no error, an empty list included when the token covers nothing
registered.

Alternatives:

- **Answer with `Rejected`.** None of its four reasons fits an answer that
  is not a refusal, and the consumer asked nothing for Lua.
- **An empty list.** The failure the specification names.

## Consequences

A consumer reading `Topics` checks the error before the list, as it does
`Schema`. The field number is 2 in both, so the two messages are read the
same way.

The `buf breaking` baseline starts at the next release, so the added field
is checked by `buf lint` and the ownership script alone until then.
