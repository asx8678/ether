// New in ether: terminal UI for the engine, built from Codex's TUI rendering modules.
//! The lifted Codex modules keep their full internal API; ether uses part of it, so they are
//! compiled with dead code and unused imports allowed. ether's own modules (`app`, `tui`, `ui_spec_cell`, `html_fence`, `html_image`, `devtools`, `view_renderer`) are not.

mod app;
mod devtools;
mod html_fence;
mod html_image;
mod kitty_diacritics;
mod tui;
mod view_renderer;
mod ui_spec_cell;

pub use app::run;
pub use html_image::terminal_is_dark;
pub use html_image::view_instructions;
pub use html_image::view_width_px;
pub use html_image::views_available;
pub use ui_spec_cell::render_ui_plain;

#[allow(dead_code, unused_imports, unused_variables)]
mod assistant_directives;
#[allow(dead_code, unused_imports, unused_variables)]
mod bottom_pane;
#[allow(dead_code, unused_imports, unused_variables)]
mod clipboard_copy;
#[allow(dead_code, unused_imports, unused_variables)]
mod color;
#[allow(dead_code, unused_imports, unused_variables)]
mod custom_terminal;
#[allow(dead_code, unused_imports, unused_variables)]
mod diff_model;
#[allow(dead_code, unused_imports, unused_variables)]
mod diff_render;
#[allow(dead_code, unused_imports, unused_variables)]
mod elapsed;
#[allow(dead_code, unused_imports, unused_variables)]
mod exec_cell;
#[allow(dead_code, unused_imports, unused_variables)]
mod exec_command;
#[allow(dead_code, unused_imports, unused_variables)]
mod git_action_directives;
#[allow(dead_code, unused_imports, unused_variables)]
mod history_cell;
#[allow(dead_code, unused_imports, unused_variables)]
mod inline_visualization;
#[allow(dead_code, unused_imports, unused_variables)]
mod insert_history;
#[allow(dead_code, unused_imports, unused_variables)]
mod key_hint;
#[allow(dead_code, unused_imports, unused_variables)]
mod keymap;
#[allow(dead_code, unused_imports, unused_variables)]
mod keymap_config;
#[allow(dead_code, unused_imports, unused_variables)]
mod line_truncation;
#[allow(dead_code, unused_imports, unused_variables)]
mod live_wrap;
#[allow(dead_code, unused_imports, unused_variables)]
mod markdown;
#[allow(dead_code, unused_imports, unused_variables)]
mod markdown_copy;
#[allow(dead_code, unused_imports, unused_variables)]
mod markdown_render;
#[allow(dead_code, unused_imports, unused_variables)]
mod markdown_stream;
#[allow(dead_code, unused_imports, unused_variables)]
mod markdown_text_merge;
#[allow(dead_code, unused_imports, unused_variables)]
mod motion;
#[allow(dead_code, unused_imports, unused_variables)]
mod render;
#[allow(dead_code, unused_imports, unused_variables)]
mod rendering_config;
#[allow(dead_code, unused_imports, unused_variables)]
mod streaming;
#[allow(dead_code, unused_imports, unused_variables)]
mod style;
#[allow(dead_code, unused_imports, unused_variables)]
mod table_detect;
#[allow(dead_code, unused_imports, unused_variables)]
mod terminal_detection;
#[allow(dead_code, unused_imports, unused_variables)]
mod terminal_hyperlinks;
#[allow(dead_code, unused_imports, unused_variables)]
mod terminal_palette;
#[allow(dead_code, unused_imports, unused_variables)]
mod terminal_probe;
#[allow(dead_code, unused_imports, unused_variables)]
mod text_selection;
#[allow(dead_code, unused_imports, unused_variables)]
mod tool_output;
#[allow(dead_code, unused_imports, unused_variables)]
mod ui_consts;
#[allow(dead_code, unused_imports, unused_variables)]
mod vim_search;
#[allow(dead_code, unused_imports, unused_variables)]
mod width;
#[allow(dead_code, unused_imports, unused_variables)]
mod wrapping;
