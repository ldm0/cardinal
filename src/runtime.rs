use once_cell::sync::Lazy;
use tokio::runtime;

static RUNTIME: Lazy<runtime::Runtime> = Lazy::new(|| {
    runtime::Builder::new_multi_thread()
        .thread_name("cardinal")
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
});

pub fn runtime() -> &'static runtime::Runtime {
    &RUNTIME
}
