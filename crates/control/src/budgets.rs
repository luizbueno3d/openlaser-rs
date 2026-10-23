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
