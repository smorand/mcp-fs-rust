---
Id: BL-0067
Title: Take the project id from the URL (path or query string) instead of a tool argument
Nature: FEAT
Created: 2026-10-19
---

## What
The project id (today the required `mount_id` argument on every `fs.*` and `git.*` tool) should be
injected by the server from the request URL and removed from the tool arguments. Preferred outcome:
the project id is either a parameter or a query string information, so a client can address a
project through the endpoint it connects to rather than repeating it in every call.

## Why
Repeating the project id in every call is noise for the LLM and a source of cross project mistakes;
binding it to the connection makes the scope explicit and stable for a session.

## Notes
Verbatim intent: "Project ID must be injected and removed from argument. Injection should come from
URL argument instead of being a parameter. Actually the best situation: the project Id is either a
parameter either a query string information."
Touches the frozen tool contract (`TOOL_CONTRACT.txt`, `tool-contract-golden.json`) and the REST
plane; both open questions (path vs query string, behaviour when the URL carries none) are for the spec.
