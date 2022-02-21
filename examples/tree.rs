extern crate cardinal;

use anyhow::{Context, Result};
use cardinal::fs_entry::DiskEntry;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    /*
    info!("cardinal starts");
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

    cardinal::init_sdk_facade();
    std::thread::sleep(std::time::Duration::from_secs_f32(500.));
    cardinal::close_sdk_facade();
    Ok(())
}
