use crate::{MetadataCache, cache::SlabNode};
use anyhow::{Context, Result};
use bincode::{Decode, Encode, config::Configuration};
use slab::Slab;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    thread::available_parallelism,
    time::Instant,
};
use tracing::info;
use typed_num::Num;

const LSF_VERSION: i64 = 1;

#[derive(Encode, Decode)]
pub struct PersistentStorage {
    pub version: Num<LSF_VERSION>,
    /// The last event id of the cache.
    pub last_event_id: u64,
    /// Root file path of the cache
    pub path: PathBuf,
    /// Root index of the slab
    pub slab_root: usize,
    pub slab: Slab<SlabNode>,
    pub name_index: BTreeMap<String, Vec<usize>>,
    pub metadata_cache: MetadataCache,
}

const BINCODE_CONDFIG: Configuration = bincode::config::standard();

pub fn read_cache_from_file(path: &Path) -> Result<PersistentStorage> {
    let cache_decode_time = Instant::now();
    let input = File::open(path).context("Failed to open cache file")?;
    let input = zstd::Decoder::new(input).context("Failed to create zstd decoder")?;
    let mut input = BufReader::new(input);
    let storage: PersistentStorage = bincode::decode_from_std_read(&mut input, BINCODE_CONDFIG)
        .context("Failed to decode cache")?;
    info!("Cache decode time: {:?}", cache_decode_time.elapsed());
    Ok(storage)
}

pub fn write_cache_to_file(path: &Path, storage: PersistentStorage) -> Result<()> {
    let cache_encode_time = Instant::now();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let tmp_path = &path.with_extension(".sctmp");
    {
        let output = File::create(tmp_path).context("Failed to create cache file")?;
        let mut output = zstd::Encoder::new(output, 6).context("Failed to create zstd encoder")?;
        output
            .multithread(available_parallelism().map(|x| x.get() as u32).unwrap_or(4))
            .context("Failed to create parallel zstd encoder")?;
        let output = output.auto_finish();
        let mut output = BufWriter::new(output);
        bincode::encode_into_std_write(
            &storage, // 使用传入的 storage
            &mut output,
            BINCODE_CONDFIG,
        )
        .context("Failed to encode cache")?;
    }
    fs::rename(tmp_path, path).context("Failed to rename cache file")?;
    info!("Cache encode time: {:?}", cache_encode_time.elapsed());
    info!(
        "Cache size: {} MB",
        fs::symlink_metadata(path)
            .context("Failed to get cache file metadata")?
            .len() as f32
            / 1024.
            / 1024.
    );
    Ok(())
}
