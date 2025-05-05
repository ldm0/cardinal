use super::SlabNode;
use anyhow::{Context, Result};
use bincode::{Decode, Encode, config::Configuration};
use slab::Slab;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::Path,
    thread::available_parallelism,
    time::Instant,
};

#[derive(Encode, Decode)]
pub struct PersistentStorage {
    // slab_root: usize, slab_root 暂时不需要，因为没有从 root 遍历的需求
    pub slab: Slab<SlabNode>,
    pub name_index: BTreeMap<String, Vec<usize>>,
}

const CACHE_PATH: &str = "target/cache.zstd";
const CACHE_TMP_PATH: &str = "target/cache.zstd.tmp";
const BINCODE_CONDFIG: Configuration = bincode::config::standard();

pub fn read_cache_from_file() -> Result<(Slab<SlabNode>, BTreeMap<String, Vec<usize>>)> {
    let cache_decode_time = Instant::now();
    let input = File::open(CACHE_PATH).context("Failed to open cache file")?;
    let input = zstd::Decoder::new(input).context("Failed to create zstd decoder")?;
    let mut input = BufReader::new(input);
    let storage: PersistentStorage = bincode::decode_from_std_read(&mut input, BINCODE_CONDFIG)
        .context("Failed to decode cache")?;
    dbg!(cache_decode_time.elapsed());
    Ok((storage.slab, storage.name_index))
}

pub fn write_cache_to_file(
    slab: Slab<SlabNode>,
    name_index: BTreeMap<String, Vec<usize>>,
) -> Result<()> {
    let cache_encode_time = Instant::now();
    {
        let output = File::create(CACHE_TMP_PATH).context("Failed to create cache file")?;
        let mut output = zstd::Encoder::new(output, 6).context("Failed to create zstd encoder")?;
        output
            .multithread(available_parallelism().map(|x| x.get() as u32).unwrap_or(4))
            .context("Failed to create parallel zstd encoder")?;
        let output = output.auto_finish();
        let mut output = BufWriter::new(output);
        bincode::encode_into_std_write(
            &PersistentStorage { slab, name_index },
            &mut output,
            BINCODE_CONDFIG,
        )
        .context("Failed to encode cache")?;
    }
    fs::rename(CACHE_TMP_PATH, CACHE_PATH).unwrap();
    dbg!(cache_encode_time.elapsed());
    dbg!(fs::metadata(CACHE_PATH).unwrap().len() / 1024 / 1024);
    Ok(())
}

pub fn cache_exists() -> bool {
    Path::new(CACHE_PATH).exists()
}
