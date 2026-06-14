//! The layout engine (§5, Stage 3): turns the styled tree into a galley — a
//! single continuous coordinate space of positioned [`Fragment`]s that the
//! fragmenter (Stage 4) paginates. Block and table flow and inline/text layout
//! are owned here; flex delegates to `taffy` (§5.3 decision).

pub mod block;
pub mod boxgen;
pub mod fragment;
pub mod inline;
pub mod value;
