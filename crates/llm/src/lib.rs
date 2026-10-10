// New in ether: model clients behind one interface (Responses API now, Chat Completions in Phase 4).
mod client;
mod common;
mod error;
mod replay;
mod responses;
mod responses_headers;
mod sse;

pub use client::ClientBuildError;
pub use client::ModelClient;
pub use client::Prompt;
pub use client::ResponsesModelClient;
pub use common::OpenAiVerbosity;
pub use common::Reasoning;
pub use common::ResponseEvent;
pub use common::ResponseStream;
pub use common::ResponsesApiRequest;
pub use error::ApiError;
pub use replay::ReplayError;
pub use replay::ReplayModelClient;
