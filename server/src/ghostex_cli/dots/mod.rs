//! `ghostex dots`: OpenDots from the command line and the phone, plus `ghostex dots mcp`, the stdio
//! MCP server every dot session loads (server/src/open_dots/).
mod command;
mod mcp;

pub(super) use command::run;
