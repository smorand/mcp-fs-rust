---
id: BL-0006
title: Extract uploaded archives in place (zip, 7z, tar/tgz/tb2/txz, rar), with password prompt when required
kind: idea
suggested_command: /spec-feat
created: 2026-10-03
origin: user
---

## What
An MCP tool (or equivalent) that runs "unzip" server-side on an archive already stored in the
FS: the archive's contents land directly as files on the filesystem, with no
download/re-upload round trip through the client. Must support at least 7z, tar, tgz/tb2/txz and
zip and rar. If the server detects the archive is password protected, it asks for the password
instead of failing.

## Why
Today an uploaded archive has to be downloaded, extracted locally and re-uploaded file by file;
doing the extraction server-side is both faster and keeps large transfers off the client.

## Notes
This is the inverse of [[BL-0005]] (zip export via signed URL). `rar` extraction has licensing/
library caveats worth checking before committing to the format list. Needs a decision on target
path (new folder named after the archive vs explicit destination) and on conflict handling when
extracted entries already exist.
