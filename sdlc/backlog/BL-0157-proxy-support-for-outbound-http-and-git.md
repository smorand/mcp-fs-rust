---
Id: BL-0157
Title: Proxy support for the server's outbound HTTP and git connections
Nature: FEAT
Source: asked by the owner on 2026-11-12
Created: 2026-11-12
---

## What
The server makes outbound calls: the web and context7 tools (`tools/web.rs`, `tools/context7.rs`), git remotes over HTTP (`git/remote.rs`, libgit2), the pull request provider clients (`git/provider/mod.rs`), the OAuth device flow (`git/oauth/device_flow.rs`), the document converter web service (`docs/service.rs`), OCR, embedding and rerank endpoints (`docs/ocr.rs`, `search/embedding.rs`, `search/rerank.rs`) and the S3 store. None has a documented proxy setting. In a corporate network where outbound traffic must go through a proxy these calls cannot work, or work only by accident of the environment.

## Why
Operators behind a proxy cannot use the web family, git remotes or hosted endpoints without changing the network. What happens today is not specified: the root `Cargo.toml` builds reqwest 0.12 with `default-features = false` and `rustls-tls`, and no code sets or disables a proxy, so whether the standard `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` variables are honored by each client is unverified, and libgit2 has its own proxy options.

## Evidence
`rg -n 'reqwest::Client' crates` lists the clients (each built separately: `web.rs:260`, `context7.rs:87`, `git/provider/mod.rs:385`, `device_flow.rs:158`, `docs/service.rs:532`, `docs/ocr.rs:60`, plain `Client::new()` in `cli.rs:210`, `app.rs:111`); `rg -n -i proxy crates/core/src` finds only two comments about proxy error pages; `Cargo.toml:110`.

## Notes
Proposal for the spec: first verify and document what each client does with the standard variables today; then one configuration section (proxy URL, no proxy list, optional credentials from the environment only, never logged) applied through one shared client builder, plus libgit2 proxy options for git remotes and the S3 client. Interplay with BL-0156: address filtering must judge the final destination, not the proxy address. Tests need a local proxy stand-in.
