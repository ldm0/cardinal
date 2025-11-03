<div align="center">
  <img src="cardinal/mac-icon_1024x1024.png" alt="Cardinal icon" width="120" height="120">
  <h1>Cardinal</h1>
  <p>A fast file searching tool for macOS.</p>
  <p>
    <a href="#requirements">Requirements</a> ·
    <a href="#running-cardinal">Running Cardinal</a> ·
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

Happy searching!
