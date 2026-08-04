//! The `/api/fs` REST surface: a bytes plane for a UI or a script (browse,
//! upload, download, zip) plus endpoint for endpoint parity with the MCP `fs.*`
//! tools, and the public OpenAPI / Swagger UI documentation of both.
//!
//! Port of the C# `Api/DataPlane.cs` and `Api/OpenApiToolDocs.cs`. Two routers
//! are exposed because they have different security postures:
//!
//! * [`router`] serves `/api/fs/**` and verifies a bearer JWT on every request,
//!   then applies the same project membership gate the tools use.
//! * [`openapi_router`] serves `/api/swagger.json` and `/api/docs`, both public,
//!   exactly like the C# (its identity middleware only guards the MCP prefix).
//!
//! Both are mounted only when `api.enabled` is true; the caller owns that check.

pub mod dataplane;
pub mod openapi;

pub use dataplane::router;
pub use openapi::openapi_router;
