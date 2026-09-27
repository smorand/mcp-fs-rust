//! Blob backends: content-addressed byte stores keyed by sha256.
//!
//! `local` writes one file per blob under `{dir}/{bucket}/`; `s3` puts one object
//! per blob in bucket `{prefix}{project_id}` with the sha256 as the key.

pub mod local;
pub mod s3;
