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

/// Authenticated connections over [`MAX_CONNECTIONS`] that are still served, each for exactly one `machine.stop`
/// exchange (within [`AUTH_DEADLINE`] of its auth). Only an authenticated connection takes one of these slots or one
/// of the `MAX_CONNECTIONS`; past both, the auth is answered with an id-null "connection limit reached".
pub const STOP_ONLY_RESERVE: usize = 4;
/// Sockets accepted but not yet authenticated: a pool of their own, separate from the authenticated slots and the
/// stop reserve. When it is full, the oldest pending socket that has sent nothing yet is closed to admit the new one
/// (the oldest of all only when every pending socket has spoken; never the newcomer refused), so a flood of silent
/// sockets cannot push out a client whose auth frame has arrived, however late its thread is scheduled.
pub const MAX_PENDING_AUTH: usize = 32;
/// How long a client keeps trying to deliver one `machine.stop`, reconnects and resends included (the bridge's
/// `StopHandle`). The server's promise is stated against this budget, not against one connection attempt: a client
/// whose bytes had not arrived yet can lose its pending slot to a churning flood, and its retry gets through.
pub const STOP_DELIVERY_BUDGET: Duration = Duration::from_secs(2);
/// The auth frame (and an optional `hello` before it) must arrive within this deadline of `accept`, and the whole
/// pre-auth exchange is bounded by it (not per read), so a silent or trickling socket cannot hold a thread for long.
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
