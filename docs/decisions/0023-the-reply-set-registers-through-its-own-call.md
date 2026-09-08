# ADR 0023: The reply set registers through its own call

## Status

Accepted

## Context

ADR 0017 has the registry hold a fourth table beside the class, route and
capability maps: the set of topics a record may be addressed to one
connection on, which the generator writes from every `reply_to` in the
schema. The specification names three calls for three tables, SPEC 5.1:

> ```
> shim.classes(table)        -- register topic -> record class. Additive; see below.
> shim.routes(table)         -- register topic -> target. Additive; see below.
> shim.caps(table)           -- register topic -> capability. Additive; see below.
> ```

and ADR 0017 left the fourth table's shape open:

> The registration surface grows by one table. SPEC 5.1 names three `shim`
> calls for three tables, and the fourth needs a call or an argument of its
> own; 2.16 chooses, and the interface version moves with it if the choice is
> a call.

The three tables map a topic to a value and merge under one rule: a row on
a new topic is added, a row repeating the map is a no-op, and a row
disagreeing with the map refuses the call whole. The reply set has no value
to disagree on. A topic is a reply or it is not, and registering it twice
says the same thing twice.

The interface version moves for the three calls whether or not a fourth is
added, so the version cost of a call is nothing here.

## Decision

`shim.replies(list)` is the fourth call. It takes a list of topics, adds
each to the addressable set, and answers how many were new. It is never
refused by the broker: the list is checked to be one of strings before the
broker sees it, and a set has no conflict to refuse.

Alternatives:

- **A second argument to `shim.classes`.** The class table and the reply
  set come from the same generated file, so one call could carry both. It
  would tie a call that can be refused whole to a table that cannot be, and
  a registrar wanting to register replies alone would have to pass an empty
  class table.
- **A reserved key inside the classes table.** A topic is a string key, and
  a key that is not a topic is what the reader refuses.
- **A class member meaning "reply".** Class is drop policy and addressing is
  delivery; ADR 0017 rejected this already.

## Consequences

The interface version is `4`: four calls were added at once, and a hook
driver comparing against `3` disables itself.

The generator emits one more table and one more call per target file, and
a hand-written registrar that forgets the call sees its typed reply refused
at `begin_to` by name, which is the failure ADR 0017 chose over a silent
fan-out.

A reply topic still needs a class and a capability: `replies` marks it
addressable and nothing more, and a `begin_to` on it is refused for a
missing class or capability the way a `begin` is.

This reopens if the addressable set ever needs a value per topic, such as
the request a reply answers. It would then be a map under the merge rule,
and this call would take a table rather than a list.
