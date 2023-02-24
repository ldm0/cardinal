use crate::consts::*;
use crate::disk_entry::DiskEntry;
use crate::fs_visitor;
use crate::fsevent::EventFlag;
use crate::fsevent::EventId;
use crate::fsevent::FsEvent;
use crate::models::DbMeta;
use crate::models::DiskEntryRaw;
use crate::schema;
use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use crossbeam_channel::bounded;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel_migrations::MigrationHarness;
use fsevent_sys::FSEventStreamEventId;
use pathbytes::p2b;
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;

fn snapshot_fs(conn: &mut CardinalDbConnection) -> Result<()> {
    let (raw_entry_sender, raw_entry_receiver) = bounded(MAX_RAW_ENTRY_COUNT);

    std::thread::spawn(move || {
        let threads = num_cpus::get_physical();
        dbg!(threads);
        let walkdir = ignore::WalkBuilder::new("/")
            .follow_links(false)
            .git_exclude(false)
            .git_global(false)
            .git_ignore(false)
            .hidden(false)
            .ignore(false)
            .ignore_case_insensitive(false)
            .max_depth(None)
            .max_filesize(None)
            .parents(false)
            .require_git(false)
            .same_file_system(true)
            .skip_stdout(false)
            .standard_filters(false)
            .threads(threads)
            .build_parallel();
        let mut visitor_builder = fs_visitor::VisitorBuilder { raw_entry_sender };
        walkdir.visit(&mut visitor_builder);
    });

    let mut last_time = Instant::now();
    let mut insert_num = 0;
    let mut printed = 0;
    for entries in raw_entry_receiver.iter() {
        if insert_num - printed >= 100000 {
            info!(
                "insert: {}, speed: {}i/s, remaining: {}",
                insert_num,
                (insert_num - printed) as f32 / last_time.elapsed().as_secs_f32(),
                raw_entry_receiver.len(),
            );
            last_time = Instant::now();
            printed = insert_num;
        }
        insert_num += entries.len();
        conn.save_entries(&entries)
            .context("Save entries failed.")?;
    }

    Ok(())
}

struct CardinalDbConnection {
    conn: SqliteConnection,
}

impl CardinalDbConnection {
    fn connect() -> Result<Self> {
        let mut conn = SqliteConnection::establish(DATABASE_URL).with_context(|| {
            anyhow!(
                "Establish sqlite connection with url: `{}` failed.",
                DATABASE_URL
            )
        })?;
        conn.batch_execute(CONNECTION_PRAGMAS)
            .context("Run connection pragmas failed.")?;
        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow!(e))
            .context("Run connection migrations failed.")?;
        Ok(Self { conn })
    }
    fn get_event_id(&mut self) -> Result<FSEventStreamEventId> {
        use schema::db_meta::dsl::*;
        let event_id = db_meta
            .select(the_value)
            .filter(the_key.eq(b"event_id".to_vec()))
            .first::<Vec<u8>>(&mut self.conn)
            .context("Get event_id failed.")?;
        bincode::decode_from_slice(&event_id, CONFIG)
            .map(|(x, _)| x)
            .context("Decode event id failed.")
    }

    fn save_event_id(&mut self, event_id: &FSEventStreamEventId) -> Result<()> {
        use schema::db_meta::dsl::*;
        let event_id =
            bincode::encode_to_vec(event_id, CONFIG).context("Encode event id failed.")?;
        let new_meta = DbMeta {
            the_key: b"event_id".to_vec(),
            the_value: event_id,
        };
        diesel::insert_into(db_meta)
            .values(&new_meta)
            .on_conflict(the_key)
            .do_update()
            .set(the_value.eq(&new_meta.the_value))
            .execute(&mut self.conn)
            .context("Upsert event id to db failed.")?;
        Ok(())
    }

    fn save_entry(&mut self, entry: &DiskEntryRaw) -> Result<()> {
        use schema::dir_entrys::dsl::*;
        diesel::insert_into(dir_entrys)
            .values(entry)
            .on_conflict(the_path)
            .do_update()
            .set(the_meta.eq(&entry.the_meta))
            .execute(&mut self.conn)
            .context("Upsert entry to db failed.")?;
        Ok(())
    }

    fn delete_entry(&mut self, path_to_delete: &str) -> Result<()> {
        use schema::dir_entrys::dsl::*;
        diesel::delete(dir_entrys.filter(the_path.eq(path_to_delete)))
            .execute(&mut self.conn)
            .context("Remove entry in db failed.")?;
        Ok(())
    }

    fn save_entries(&mut self, entries: &[DiskEntryRaw]) -> Result<()> {
        self.conn
            .transaction(|conn| {
                use schema::dir_entrys::dsl::*;
                for entry in entries.iter() {
                    let _num_insert = diesel::insert_into(dir_entrys)
                        .values(entry)
                        .on_conflict(the_path)
                        .do_update()
                        .set(the_meta.eq(&entry.the_meta))
                        .execute(conn)?;
                }
                Ok::<(), diesel::result::Error>(())
            })
            .context("Batch save entries failed.")
    }

    fn fuzz_search(&mut self, filter: &str) -> Result<Vec<String>> {
        use schema::dir_entrys::dsl::*;
        dir_entrys
            .select(the_path)
            .filter(the_path.like(format!("%{}%", filter)))
            .load(&mut self.conn)
            .context("Fuzz search failed.")
    }
}

/// The Database contains the file system snapshot and the time starting
/// to take the snapshot.
///
/// To make it really useful, merge the filesystem change(from start time to
/// current time) into the file system.
pub struct Database {
    /// The time starting to scan this file system tree.
    pub event_id: FSEventStreamEventId,
    conn: CardinalDbConnection,
}

impl Database {
    pub fn from_fs() -> Result<Self> {
        let mut conn = CardinalDbConnection::connect().context("Get db connection failed.")?;
        let event_id = match conn.get_event_id() {
            Ok(x) => x,
            Err(e) => {
                info!(?e, "Event id fetching failed:");
                // scan_fs needs a lot of time, so event id should be gotten before it.
                let new_event_id = EventId::now();
                snapshot_fs(&mut conn).context("Scan fs failed.")?;
                conn.save_event_id(&new_event_id.raw_event_id)
                    .context("Save current event id failed")?;
                new_event_id.raw_event_id
            }
        };

        info!(?event_id, "The start event id");
        Ok(Self { event_id, conn })
    }

    pub fn merge_event(&mut self, fs_event: FsEvent) -> Result<()> {
        match fs_event.flag {
            EventFlag::Delete | EventFlag::Modify | EventFlag::Create => {
                let path = &fs_event.path;
                match std::fs::metadata(&path) {
                    Ok(metadata) => {
                        let entry = DiskEntry {
                            path: fs_event.path,
                            meta: metadata.into(),
                        }
                        .to_raw()
                        .context("Encode meta failed.")?;
                        self.conn.save_entry(&entry).context("Save entry failed.")?;
                    }
                    Err(_e) => {
                        self.conn
                            .delete_entry(&path.to_string_lossy())
                            .context("Delete entry failed.")?;
                    }
                }
            }
        }
        self.conn
            .save_event_id(&fs_event.id)
            .context("Save event id failed.")
    }

    pub fn search(&mut self, filter: &str) -> Result<Vec<String>> {
        self.conn.fuzz_search(filter)
    }
}
