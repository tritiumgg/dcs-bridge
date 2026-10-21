# ADR 0073: doctor finds the write directory under the home directory

## Status

Accepted

## Context

The specification defines the write directory as the Saved Games DCS
folder, resolved from `lfs.writedir()` (SPEC §5.1.1), and every Lua file the
bridge ships resolves it that way. `dcsb doctor` runs outside DCS, with no `lfs` to ask. It still has to find
the tree a release was extracted over, and the user running it after a
failed start may not know which of DCS's two Saved Games names their build
uses.

## Decision

`doctor` takes the write directory from `--dir`, and without it looks under
the home directory for `Saved Games\DCS` and `Saved Games\DCS.openbeta`.

- **A candidate counts when it holds the loader,**
  `Scripts\Hooks\DCSBridge.lua`. A Saved Games folder with no loader is a
  DCS install the bridge was never extracted over.
- **Every candidate that counts is checked**, one after the other, since a
  machine can hold both builds.
- **The home directory is `USERPROFILE`,** read in `main` and passed to the
  discovery as a parameter, so a test names a scratch directory on any
  host and never reaches the machine's own Saved Games tree.
- **No candidate is exit 2.** The message names both paths it looked at and
  says `--dir` names another. `--dir` naming no directory is exit 2 too.

Alternatives:

- **Require `--dir`.** The common install would need a path typed out to
  diagnose it, and the user most likely to run `doctor` is the one who
  does not know where things went.
- **Read the write directory from the registry or DCS's own settings.** No
  file in the tree records where DCS reads that, and a guess at a registry
  key would be a second place to go wrong.

## Consequences

A write directory moved with a symbolic link or a Windows folder
redirection is found when `USERPROFILE` points at it, and otherwise needs
`--dir`. On macOS and Linux, where `USERPROFILE` is normally unset, `doctor`
runs only with `--dir`, which is what a contributor checking a scratch tree
passes anyway.
