//! One module per hook subcommand, plus the parsers they share.

pub mod attribution;
pub mod context_inject;
pub mod design_mockup;
pub mod git_command;
pub mod git_identity;
pub mod grill;
mod grill_text;
pub mod herdr_session;
pub mod image_tools;
pub mod pr_detect;
pub mod quality_gate;
pub mod transcript;
