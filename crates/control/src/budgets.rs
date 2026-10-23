//! PhotoCraft's constants plus AUT B12.

use std::time::Duration;

pub const MAX_REQUEST_BYTES: usize = 1 << 20;
pub const MAX_RESPONSE_BYTES: usize = 8 << 20;
pub const MAX_PNG_BYTES: usize = 5 << 20;
pub const PREVIEW_DEFAULT_SIDE: u32 = 1024;
pub const PREVIEW_MAX_SIDE: u32 = 2048;
pub const BATCH_MAX_STEPS: usize = 256;
pub const MAX_CONNECTIONS: usize = 16;
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const APP_DEADLINE: Duration = Duration::from_secs(60);
pub const BRIDGE_TIMEOUT: Duration = Duration::from_secs(55);
pub const RECONNECT_IDLE: Duration = Duration::from_secs(25);
pub const STOP_WAIT: Duration = Duration::from_millis(500);
pub const SIM_LISTING_MAX_ROWS: usize = 2000;
pub const SIM_LISTING_MAX_BYTES: usize = 256 << 10;
pub const INSPECT_OBJECTS_DEFAULT: usize = 200;
pub const NOTE_MAX_CHARS: usize = 280;
pub const SUBMISSION_TTL: Duration = Duration::from_secs(900);
pub const FRAME_REQUEST_TTL: Duration = Duration::from_secs(120);

// ---- WP7 additions (AUT A9/A10, review B7 and the security items) ------------------------------------------------

/// Connections over [`MAX_CONNECTIONS`] that are still accepted, each for exactly one `auth` + `machine.stop`
/// exchange. Every live socket counts from `accept`, authenticated or not; past this reserve a connection is refused.
pub const STOP_ONLY_RESERVE: usize = 4;
/// The auth frame (and an optional `hello` before it) must arrive within this deadline of `accept`, and the whole
/// pre-auth exchange is bounded by it (not per read), so a silent or trickling socket cannot hold a slot.
pub const AUTH_DEADLINE: Duration = Duration::from_secs(2);
/// The longest pre-auth frame (`hello` or `auth`).
pub const AUTH_MAX_BYTES: usize = 4 << 10;
/// Client name and version shown in the GUI are cut to this many characters (control characters removed).
pub const CLIENT_NAME_MAX_CHARS: usize = 64;
/// The largest file `AuthorizedWorkspace::read` returns.
pub const MAX_READ_BYTES: u64 = 256 << 20;
/// A control token file is 64 hex digits and a newline; anything much longer is not one.
pub const TOKEN_FILE_MAX_BYTES: u64 = 1 << 10;
/// The discovery file is a small JSON object.
pub const DISCOVERY_MAX_BYTES: u64 = 64 << 10;
/// Bridged PNG headers are checked against these before the image is handed to an agent (AUT A10).
pub const SCREENSHOT_MAX_EDGE: u32 = 8192;
pub const SCREENSHOT_MAX_PIXELS: u64 = 16 << 20;
