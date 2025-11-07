<div align="center">
  <img src="cardinal/mac-icon_1024x1024.png" alt="Cardinal icon" width="120" height="120">
  <h1>Cardinal</h1>
  <p>A fast file searching tool for macOS.</p>
  <p>
    <a href="#using-cardinal">Using Cardinal</a> ·
    <a href="#building-cardinal">Building Cardinal</a>
  </p>
  <img src="doc/UI.gif" alt="Cardinal UI preview" width="720">
</div>

---

## Using Cardinal

### Download

Grab the latest packaged builds from [GitHub Releases](https://github.com/ldm0/cardinal/releases/).

### i18n support

Need a different language? Click the 🌍 button in the status bar to switch instantly.

### Search basics

Cardinal's search box understands several pattern styles to help you locate files fast:

- `substr` – return files whose names contain `substr` as a contiguous fragment.
- `/prefix` – keep files whose names start with `prefix`.
- `suffix/` – match files whose names end with `suffix`.
- `/exact/` – only list files that exactly match `exact`.
- `a/part/of/path` – search for substrings anywhere in the path, letting you mix directory and filename segments.

Both **case-sensitive** and **regular-expression** modes are available via the toggles next to the search input, so you can tighten queries whenever needed.

### Quick Look previews

Click any result row to select it, then tap the space bar to launch a native Quick Look preview of that file without leaving Cardinal.

### Global hide/show hotkey

Hit `Cmd+Shift+Space` from anywhere to toggle the Cardinal window. The shortcut instantly pops Cardinal to the front for a new search, and pressing it again hides the window while the app keeps running in the background tray.

Happy searching!

---

## Building Cardinal

### Requirements

- macOS 12+
- Rust toolchain
- Node.js 18+ with npm
- Xcode command-line tools & Tauri prerequisites (<https://tauri.app/start/prerequisites/>)

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
