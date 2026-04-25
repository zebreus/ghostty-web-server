//! Compile-time configuration. Edit and rebuild to change behavior. Mirrors
//! `src/settings.ts` from the original implementation.

/// After the user stops resizing for this many ms, send Ctrl+L so bash
/// repaints the prompt from scratch. 0 disables.
pub const RESIZE_AUTO_REDRAW_MS: u32 = 0;

/// Minimum interval between WebSocket resize messages while the user is
/// dragging.
pub const RESIZE_MIN_INTERVAL_MS: u32 = 50;

/// Floor for terminal dimensions.
pub const MIN_COLS: u32 = 20;
pub const MIN_ROWS: u32 = 4;

/// Forward mouse events to PTY when the program enables tracking.
pub const MOUSE_ENABLED: bool = true;

/// Wrap pasted text with bracketed-paste markers when `?2004h` is on.
pub const BRACKETED_PASTE_ENABLED: bool = true;

/// Forward window focus/blur as `\e[I`/`\e[O` when `?1004h` is on.
pub const FOCUS_EVENTS_ENABLED: bool = true;

/// Reflect terminal title escapes into `document.title`.
pub const SET_DOCUMENT_TITLE: bool = true;

/// Brief CSS flash on `\a`.
pub const VISUAL_BELL_ENABLED: bool = true;

/// Short 800 Hz beep on `\a` via Web Audio.
pub const AUDIBLE_BELL_ENABLED: bool = true;

/// Log "ding" to console on every bell.
pub const DEBUG_BELL_ENABLED: bool = true;
