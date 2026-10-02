---
id: BL-0005
title: Export a set of files as a zip via a signed download URL
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
Let a caller select a bunch of files from the FS and export them as a single zip. The result is
made available through a signed URL that can be used to download the archive, rather than
streaming the zip bytes back inline.

## Why
Downloading many files one by one through existing tools is slow and chatty; a single signed link
to a prebuilt zip is the natural way to hand off a bulk export.

## Notes
Needs a decision on where the generated zip is stored (blob store under a throwaway key?) and
the signed URL's expiry/revocation mechanism; the project already has `zip` as a dependency.
