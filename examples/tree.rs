use cardinal::fs_entry::Volumes;

extern crate cardinal;

fn main() {
    let time = std::time::Instant::now();
    let hierarchy = cardinal::fs_entry::Volumes::from_fs().unwrap();
    println!("num: {}", hierarchy.entries.len());
    println!("elapsed: {}s", time.elapsed().as_secs_f32());
    hierarchy.to_fs().unwrap();
    println!("elapsed: {}s", time.elapsed().as_secs_f32());
    /*
    cardinal::init_sdk();
    loop {
        std::thread::sleep(std::time::Duration::from_secs_f32(0.5));
        let events = cardinal::take_fs_events();
        if !events.is_empty() {
            println!("{:#?}", events);
        }
    }
     */
}
