pub use libc::{ENOENT, ENOSYS};
pub use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub use anyhow::{anyhow, Result};
#[allow(unused)]
pub use fuser::{FileType, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request};
#[allow(unused)]
pub use log::{debug, error, info, warn, LevelFilter};
pub use serde::{Deserialize, Serialize};

pub use crate::Lis;

pub type Inode = u64;
pub type FileHandle = u64;

pub const BLOCK_SIZE: u64 = 512;
pub const MAX_NAME_LENGTH: u32 = 255;
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024 * 1024;

// Top two file handle bits are used to store permissions
// Note: This isn't safe, since the client can modify those bits.
pub const FILE_HANDLE_READ_BIT: u64 = 1 << 63;
pub const FILE_HANDLE_WRITE_BIT: u64 = 1 << 62;

pub const FMODE_EXEC: i32 = 0x20;
