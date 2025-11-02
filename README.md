<div align="center">
  <img src="cardinal/mac-icon_1024x1024.png" alt="Cardinal icon" width="120" height="120">
  <h1>Cardinal</h1>
  <p>Lightning-fast file search for macOS with live previews and fuzzy matching.</p>
  <p>
    <a href="#requirements">Requirements</a> ·
    <a href="#running-cardinal">Running Cardinal</a> ·
    <a href="#testing--quality">Testing &amp; Quality</a> ·
    <a href="#contributing">Contributing</a>
  </p>
  <img src="doc/UI.gif" alt="Cardinal UI preview" width="720">
</div>

---

## Requirements

- macOS 13+ (current integrations target macOS; other platforms are experimental)
- Rust toolchain
- Node.js 18+ with npm
- Xcode command-line tools & Tauri prerequisites (<https://tauri.app/start/prerequisites/>)

---

## Running Cardinal

### Development mode

```bash
cd cardinal
npm run tauri dev -- --release --features dev
```

### Production build

```bash
cd cardinal
npm run tauri build
```

---

## Testing & Quality

Please run these before opening a pull request or cutting a release:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -D warnings
cargo test --workspace

cd cardinal
npm run format
npm run build
```

For performance or rendering-sensitive changes, follow the profiling checklist in `doc/testing.md` (FPS capture, Safari/Chrome traces).

---

## Contributing

We welcome issues, feature requests, and PRs. Start with:

- [CONTRIBUTING.md](./CONTRIBUTING.md) for workflow expectations
- [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) for community guidelines

When filing issues, include platform details and reproduction steps. For performance regressions, attach profiler traces or screen recordings if possible.

---

## License

Cardinal is released under the [MIT License](./LICENSE).

---

Happy searching!
