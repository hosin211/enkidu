//! The Work page (phase 5; DESIGN.md, "The Work page"): your tickets and PRs as a board, read from the core's Work
//! index. What it shows is worked out in `enkidu_model::work`; here are its store and its views.

pub mod board;
pub mod card;
mod chain;
pub mod check;
mod claude_box;
mod composer;
pub mod drafts;
pub mod flow_editor;
pub mod flows_page;
mod header;
mod needs;
pub mod own_task;
pub mod page;
mod pr_tab;
pub mod queued;
mod report;
pub mod rich;
pub mod send;
pub mod shots;
pub mod store;
