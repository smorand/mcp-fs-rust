---
Id: BL-0156
Title: Refuse loopback, private and cloud metadata addresses in web.fetch and web.download
Nature: FEAT
Source: SPEC-0093 class sweep (CWE-918), validated by the owner on 2026-11-12
Created: 2026-11-12
---

## What
`web.fetch` and `web.download` (`crates/core/src/tools/web.rs`, `fetch_raw`) request any URL a member gives them. A member can therefore make the server call loopback services (`http://127.0.0.1:...`), hosts of the private networks (`10.x`, `172.16.x`, `192.168.x`, IPv6 unique local and link local addresses) and the cloud metadata endpoint (`169.254.169.254`, where cloud credentials live), and read or save the answer into the project (server side request forgery). Redirects are followed, so a public URL can also send the server to such an address.

## Why
The web tool family is off by default, so nothing is reachable in the default configuration; the exposure starts the day an operator enables it. SPEC-0093 bounded the size of the answers and left the question of which addresses a member may reach out of scope.

## Evidence
`web.rs` `build_client` (reqwest client with a timeout and a user agent, no address policy, default redirect policy); SPEC-0093 class sweep and its side findings; the stand-ins of `tools/web_testkit.rs` listen on 127.0.0.1, which the tests rely on (a test seam would be needed).

## Notes
Proposal for the spec: resolve the host and refuse every address in the blocked ranges before connecting, and again for each redirect hop (a custom redirect policy), so a DNS answer that changes between the check and the connection cannot bypass it (connect to the checked address). An operator allow list for internal hosts that are meant to be reachable, and a refusal message that names the host without leaking the resolved address. The tests need a way to reach the local stand-ins (test only configuration). The same policy would apply to the context7 tools and to any future fetch tool.
