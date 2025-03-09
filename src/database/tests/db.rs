use crate::consts::DB_PATH;
use crate::database::Database;
use crate::fs_entry::{DiskEntry, FileType, Metadata};
use crate::fsevent::EventId;
use anyhow::{Context, Result};
use pathbytes::b2p;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Serialize, Deserialize)]
struct ReadableMetadata {
    file_type: FileType,
    len: u64,
    created: String,
    modified: String,
    accessed: String,
    permissions_read_only: bool,
}

impl From<&Metadata> for ReadableMetadata {
    fn from(metadata: &Metadata) -> Self {
        Self {
            file_type: metadata.file_type,
            len: metadata.len,
            created: OffsetDateTime::from(metadata.created)
                .format(&Rfc3339)
                .unwrap(),
            modified: OffsetDateTime::from(metadata.modified)
                .format(&Rfc3339)
                .unwrap(),
            accessed: OffsetDateTime::from(metadata.accessed)
                .format(&Rfc3339)
                .unwrap(),
            permissions_read_only: metadata.permissions_read_only,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
// TODO(ldm) distinguish between macOS and Windows in field, due to the WTF-8;
struct ReadableDatabase {
    /// The snapshot time of this file system tree.
    time: EventId,
    /// Snapshot of the file system tree.
    nodes: Vec<(PathBuf, Option<ReadableMetadata>)>,
}

fn flatten_fs_entry(database: &Database) -> Result<ReadableDatabase> {
    let mut output = ReadableDatabase::default();
    output.time = database.time;
    let path = Path::new("/");
    inner(path, &database.fs_entry, &mut output);
    Ok(output)
}

/// prefix doesn't contains entry.name
fn inner(prefix: &Path, entry: &DiskEntry, output: &mut ReadableDatabase) {
    output.nodes.push((
        prefix.to_path_buf(),
        entry.metadata.as_ref().map(Into::into),
    ));
    for (name, entry) in entry.entries.iter() {
        let prefix = prefix.join(unsafe { b2p(name) });
        inner(&prefix, entry, output);
    }
}

#[test]
#[ignore]
/// Only used for checking database contents, it's placed in tests due to the
/// private field accessing.
fn db() -> Result<()> {
    const READABLE_DB_PATH: &str = "target/nodes.json";
    const PATHS_PATH: &str = "target/paths.json";

    tracing_subscriber::fmt().with_env_filter("info").init();
    let database =
        Database::from_fs(Path::new(DB_PATH)).context("Deserialize database from_fs failed")?;
    let readable_database = flatten_fs_entry(&database).context("Flatten database failed.")?;

    let db_json = serde_json::to_vec_pretty(&readable_database).context("Serialize failed.")?;
    fs::write(READABLE_DB_PATH, db_json).context("Write db json failed.")?;

    let paths: Vec<_> = readable_database
        .nodes
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    let paths_json = serde_json::to_vec_pretty(&paths).context("Serialize failed.")?;
    fs::write(PATHS_PATH, paths_json).context("Write paths json failed.")?;

    /*
    let time = std::time::Instant::now();
    let hierarchy = DiskEntry::from_fs(Path::new("/"));
    info!("elapsed: {}s", time.elapsed().as_secs_f32());

    let file = File::create("target/fs.db").context("open hierarchy file failed.")?;
    let mut file = BufWriter::new(file);

    let time = std::time::Instant::now();
    bincode::encode_into_std_write(hierarchy, &mut file, bincode::config::standard())
        .context("write hierarchy to file failed.")?;
    info!("elapsed: {}s", time.elapsed().as_secs_f32());
    */
    Ok(())
}
