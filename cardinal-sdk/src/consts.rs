use crate::models::DiskEntryRaw;
use diesel_migrations::{embed_migrations, EmbeddedMigrations};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../migrations");
pub const CONNECTION_PRAGMAS: &str =
    "PRAGMA synchronous = OFF; PRAGMA journal_mode = WAL; PRAGMA temp_store = MEMORY;";
pub const CHUNK_SIZE: usize = 1000;
pub const MAX_RAW_ENTRY_SIZE: usize = 5 * 1024 * 1024;
pub const MAX_RAW_ENTRY_COUNT: usize =
    MAX_RAW_ENTRY_SIZE / std::mem::size_of::<DiskEntryRaw>() / CHUNK_SIZE;
pub const CONFIG: bincode::config::Configuration = bincode::config::standard();

pub const DATABASE_URL: &str = std::env!("DATABASE_URL");
