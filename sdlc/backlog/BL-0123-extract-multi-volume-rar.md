---
Id: BL-0123
Title: fs.extract_archive supports multi volume RAR archives
Nature: FEAT
Source: deferred from SPEC-0080
Created: 2026-11-04
---

## What
Extract a RAR archive split over several stored files (`name.part1.rar`, `name.part2.rar`, ...), once the member points at the first part and every part sits in the same folder.

## Why
SPEC-0080 refuses `name.partN.rar` with `ERR_NOT_SUPPORTED` because the tool receives one stored file.

## Evidence
SPEC-0080 FR-NEW-010 and its scope section 3.2.

## Notes
Open product questions: how missing parts are reported, whether parts are read from the same folder only, the quota and pre scan rules across parts.
