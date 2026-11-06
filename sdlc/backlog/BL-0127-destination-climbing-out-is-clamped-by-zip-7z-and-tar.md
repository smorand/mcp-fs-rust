---
Id: BL-0127
Title: A destination such as ../../x is clamped to /x by zip, 7z and tar extraction instead of refused
Nature: FEAT
Source: found while implementing US-0164 (SPEC-0081)
Created: 2026-11-05
---

## What
`fs.extract_archive` with `destination: "../../x"` on a zip, 7z or tar archive extracts into `/x`: `SafetyManager::normalize_path` turns the relative `../../x` into `/../../x` and `PosixPath::normpath` clamps it at the root. For a bare `.gz` file SPEC-0081 requires `ERR_PATH_OUT_OF_BOUNDS` with `path escapes the volume root: ../../x`; US-0164 implements that locally in `archive_single.rs` (`normalize_destination`), so the formats now differ.

## Why
One parameter, two behaviors. A member who mistypes a relative destination writes into the project root with zip and gets a refusal with gzip.

## Evidence
`crates/core/src/safety.rs:99-111` (`normalize_path` checks `starts_with("/..")` after normalizing), `crates/core/src/util/posix.rs:115` (a test pins the clamping), `crates/core/src/tools/archive.rs` destination handling (`state.safety.normalize_path(d)?`). Observed: E2E-035 first failed with `archive is corrupt...` because `../../x` normalized to `/x` and no refusal happened.

## Notes
Decision needed: change every format to the refusal (a behavior change for zip, 7z, tar, so a spec), or document the clamping for the archive formats and keep the bare file rule. Not changed by SPEC-0081.
