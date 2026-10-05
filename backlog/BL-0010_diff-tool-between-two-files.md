---
id: BL-0010
title: Add diff tool between two text files
kind: idea
suggested_command: /spec-feat
created: 2026-10-05
origin: user
---

## What
A new tool that computes a diff between two files given their paths (presumably two `mount_id`/path
pairs, or two paths within the simulated filesystem). Only text files are supported; binary files
are out of scope.

## Why
Lets an agent compare two versions of a file (or two files) without pulling both contents and diffing
client side.

## Notes
`core/diff.rs` already exists in this project; check whether it already covers this or is a
different kind of diff before specifying.
