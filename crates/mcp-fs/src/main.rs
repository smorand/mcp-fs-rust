//! Binary entry point. All the logic lives in the library so the integration
//! tests and the parity harness can drive the same code paths.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    mcp_fs::cli::run().await
}
