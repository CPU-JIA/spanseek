//! External adapters over the stable `spanseek.external/v1` contract.
pub mod http;
pub mod mcp;
pub mod report_export;
pub mod service;
pub mod tokens;

pub use http::serve_http;
pub use mcp::{McpSession, serve_stdio};
pub use service::{ExternalService, ServicePolicy};
pub use tokens::{TokenRecord, TokenStore, TokenSummary};
