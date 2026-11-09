# MacroFlow

A smart macro builder for **macOS and Windows**, written in Rust with [GPUI](https://www.gpui.rs) (the UI framework behind the Zed editor).

You build macros as a **flowchart**: each step is a box, you drag lines between boxes, and
some steps (like **Find image**, **Window check** and **Read text**) branch two ways. You can record a whole macro, record a
single action, or build one step by step, and you can edit every delay afterwards.

```
 ┌───────┐    ┌──────────────┐ found  ┌──────────────────┐    ┌─────┐
 │ Start │───▶│  Find image  │───────▶│ Click found image│───▶│ End │
 └───────┘    │ "Submit" btn │        └──────────────────┘    └─────┘
              └──────────────┘ not found ┌───────────┐
                      ▲──────────────────│ Wait 1 s  │
                                         └───────────┘
```

Every push is built on macOS and Windows by GitHub Actions (`.github/workflows/build.yml`).
Built apps can be downloaded from each run's **Artifacts**.

## Setup

You need **Rust 1.85 or newer** (`rustup update stable`) on both platforms.

### macOS

1. Install the Xcode Command Line Tools. They provide the linker, `libclang`, `git` and the `swiftc` compiler used for the Read text helper:
   ```sh
   xcode-select --install
   ```
   You don't need the full Xcode app, because the project turns on GPUI's `runtime_shaders` feature.
2. Build and run:
   ```sh
   cd ~/Documents/macroflow
   cargo run --release
   ```
3. **Permissions.** Grant these to the app you run MacroFlow from (Terminal, iTerm, VS Code, …). Then **quit and reopen** that app:

   | Permission | Needed for | Where |
   |---|---|---|
   | **Accessibility** | Recording, playback, picking positions | System Settings → Privacy & Security → Accessibility |
   | **Input Monitoring** (if asked) | Recording keys | System Settings → Privacy & Security → Input Monitoring |
   | **Screen Recording** | Find image, Read text, Select area, window titles | System Settings → Privacy & Security → Screen & System Audio Recording |

### Windows 10 / 11

1. Install **Visual Studio Build Tools** with the **"Desktop development with C++"** workload, which includes the Windows SDK. GPUI needs the MSVC linker, and the SDK's `fxc.exe` shader compiler for release builds. If the build can't find `fxc.exe`, set `GPUI_FXC_PATH` to it, e.g. `C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\fxc.exe`.
2. Install Rust with the MSVC toolchain from <https://rustup.rs>.
3. Build and run:
   ```powershell
   cd $HOME\Documents\macroflow
   cargo run --release
   ```
4. No permissions are needed. To automate an app that runs **as administrator**, run MacroFlow as administrator too, because Windows blocks input from normal apps to admin apps.
5. Read text uses the built-in Windows OCR. It needs the OCR language for your display language, which is usually already installed. If not, add it under Settings → Time & language → Language & region.

The first build downloads and compiles GPUI, which takes a few minutes. Use `--release` (image search is much faster).

### Platform differences

| | macOS | Windows |
|---|---|---|
| Coordinates | points (Retina screenshots are scaled down) | physical pixels (MacroFlow is DPI-aware) |
| Read text (OCR) | Apple Vision via `helpers/ocr.swift` (compiled by `build.rs`) | Windows.Media.Ocr via `helpers/ocr.ps1` (embedded, run through PowerShell) |
| "cmd" in key combos | ⌘ Command | ⊞ Windows key |
| "alt" in key combos | ⌥ Option | Alt |
| App shortcuts | ⌘S, ⌘O, ⌘R… | Ctrl+S, Ctrl+O, Ctrl+R… |
| Hiding during Select area | MacroFlow hides | MacroFlow minimises |
| Bring window to front | `open -a <app>` | `WScript.Shell.AppActivate` |

Macro files are the same on both platforms. Keys are saved by name, but **screen positions and areas are specific to your screen layout**, so re-pick them on a different computer.

## Using it

| Area | What it does |
|---|---|
| **Title bar** | MacroFlow's own dark title bar: the app name (top left), show/hide panel, New / Open / Save, the macro name (click it to rename), Mouse moves toggle, **● Record**, **◎ Record step** and **▶ Run** / **■ Stop**. On macOS the traffic lights sit in it. On Windows it has minimise / maximise / close buttons. Drag empty space to move the window. |
| **Left panel** | Three tabs. **Steps**: every step type, grouped (Mouse, Keyboard, Smart checks, Flow & timing, Recording). Click one to add it. **Edit**: settings for the selected step (opens automatically when you click a step). **Macro**: macro-wide settings (name, speed, repeat, start delay, bulk delay tools). ⌘B / Ctrl+B shows or hides the panel. |
| **Canvas** | Drag steps to move them. Drag from a step's **right-hand dot** to another step to connect them. Click the **×** on a line, or right-click a dot, to disconnect. Drag empty space or scroll to pan. |
| **Status bar** | Shows what's happening and which step is running. The running step is outlined in yellow. |

If a step is selected, new steps are placed after it and connected automatically.
If you close the window or quit with unsaved changes, MacroFlow asks whether to **Save**, **Don't Save** or **Cancel**.

**Esc stops a recording or a running macro**, even when MacroFlow isn't the active window.

### Step types

| Step | Outputs | Notes |
|---|---|---|
| **Start** | next | Every run begins here. |
| **Click** | next | Left / right / middle, single or double click, at a fixed spot, at the found image (+ offset), or wherever the pointer is. "Pick position on screen" captures the pointer position after 3 s. |
| **Move mouse** | next | Same targeting options as Click. |
| **Key press** | next | Any key plus ⌃⌥⇧⌘. Use "Capture key from keyboard" to set it by pressing the keys. |
| **Type text** | next | Types a string, optionally followed by Return. |
| **Wait** | next | Fixed wait plus optional random extra time. |
| **Scroll** | next | Scrolls up/down or left/right by a number of lines. |
| **Recording** | next | A recorded sequence of raw events. Each event has its own delay, which you can edit, nudge with −/+ (Shift-click for ×10), or delete. You can also halve or double all delays, set them all to one value, remove mouse moves, or change the playback speed. |
| **Find image** | **found** / **not found** | Looks for an image on every display until it appears or the timeout runs out. **Drag a box on screen** around what to find, snip the area around the pointer, or choose a PNG. **Look in → One area** limits the search to part of the screen you drag a box around: much faster, and it can't match the same picture somewhere else. "Test now" checks it straight away. You can make it click the image when found. Later Click / Move steps can target the found image. |
| **Window check** | **yes** / **no** | Checks whether the **current window** (the front window of the app you're using) or **any open window** has a title / app name that contains, equals or starts with some text. Optional match case, "keep checking for N ms" (to wait for a window to open or come to the front) and "bring it to the front if found". **Show open windows** lists what's open right now. Click one to fill in the text. |
| **Read text** | **match** / **no match** | Reads the text in a screen area using the OCR built into the system (Apple Vision on macOS, Windows OCR on Windows). Each read cleans the image up (enlarges it, boosts contrast, flips light-on-dark text, adds a margin), reads three versions at once and takes the answer they agree on, re-reading if they don't. Number comparisons (and **Only read a number**) fix common misreads (O→0, l→1, S→5), join split digits and handle thousands separators. Click **Select area (drag a box)**: MacroFlow minimises and you drag a box around the text or number anywhere on your screen. Compare as **text** (contains, is exactly, starts with, matches regex, has any text, has changed) or as a **number** (=, ≠, >, ≥, <, ≤, using the first number in the area). "Read now" shows what it reads. "Keep reading for N ms" waits for the condition. A Type text step can type what was read with `{text}`. |
| **Group** | next | A collapsed run of steps that shows as one step (a little stack of cards). Double-click it to open it. See **Groups** below. |
| **Loop** | **repeat** / **done** | Runs the "repeat" branch N times (0 = forever), then follows "done". Connect the last step of the loop back into the Loop step. |
| **End** | none | Stops the macro. A step with nothing connected also stops it. |

Every step also has a **Note** and a **Delay before** field.

### Groups

A group turns a long run of steps into one step, so a big macro stays readable.

- **Make one from existing steps:** click a step and a bar appears at the bottom of the canvas: **▤ Group these** groups it and every step after it that runs in a straight line. To pick exactly which steps, **Shift-click** them or **Shift-drag** a box around them on the canvas, then click **▤ Group** in that bar (or press ⌘G / Ctrl+G, or use the Edit tab). Start / Group start / Group end steps are left out automatically. The steps must be one straight line, each connected to the next. A branching step (like Find image) can only be the last one.
- **Open it:** double-click the group (or select it and press Enter, or click **Open group**). Inside, the canvas is framed and tinted in the group colour, and a bar at the top shows **INSIDE GROUP** and the path (e.g. *My macro › Group 1*). Click any part of the path, **← Back**, or press **Esc** to go back out.
- **Inside a group:** it has a **Group start** and a **Group end** step. Edit, add, connect and record steps there as normal. Reaching Group end, or a step with nothing connected, leaves the group and carries on from the Group step's output. An End step inside a group still stops the whole macro.
- **Ungroup:** select the group and click **Ungroup** (or ⇧⌘G / Ctrl+Shift+G) to put its steps back in place.
- Groups can contain other groups. Changing every delay at once also changes delays inside groups. Duplicating a group copies everything in it.

### Saved file compatibility

Macros saved by older versions of MacroFlow still open: the file layout hasn't changed, only new step types were added. A macro that *uses* groups needs this version or newer to open.

### Selecting an area (drag a box)

**Select area** (Read text) and **Drag a box on screen** (Find image) work like the Windows **Snipping Tool** (Win+Shift+S):

1. MacroFlow minimises and the screen **freezes and dims**, with a small toolbar at the **top centre** and a crosshair cursor.
2. **Drag a rectangle** anywhere on the screen. The area inside it shows at full brightness with a thin white border and its size (e.g. `240 × 32`).
3. **Let go to confirm.** The box is used straight away and MacroFlow comes back.
4. **Esc** or the toolbar's **✕** cancels. A right-click while dragging throws the box away so you can start again.

**Cancelling:** Esc, the toolbar's **✕**, a right-click (when you're not drawing a box), MacroFlow's **■ Stop** button, or clicking back on MacroFlow (taskbar / Dock) all cancel. After 3 minutes it gives up on its own.

It covers the display the mouse is on. **Shift-click** the button to get 3 seconds to switch windows (or move the mouse to another display) first. On macOS the selector is a small native helper (`helpers/select.swift`, built by `build.rs`). On Windows it's a WinForms window run through PowerShell (`helpers/select.ps1`, embedded in the app). The first time on Windows it takes a second or two to start; after that it's quicker.

### Example: wait for a game/app number, then act

```
Start → Window check (current window title contains "Calculator") ──yes──▶ Read text (number ≥ 100, keep reading 30 s) ──match──▶ Click …
                                     └──no──▶ End                                       └──no match──▶ Type text "{text} wasn't enough"
```

### Changing delays for the whole macro

Select nothing (click empty canvas) to open the **Macro** settings:

- **Speed (%)**: 200% plays everything twice as fast and 50% plays at half speed. This doesn't change the stored delays.
- **× 0.5 / × 0.75 / × 1.5 / × 2**: permanently scales every delay (waits, recordings, step delays).
- **Set every recorded & step delay to …**: sets recorded event delays and step delays to one fixed value. Wait steps are left alone.
- **Gap between steps**, **Start delay** and **Repeat whole macro** (0 = until Esc).

### Shortcuts

| | macOS | Windows |
|---|---|---|
| Save / Save as | ⌘S / ⇧⌘S | Ctrl+S / Ctrl+Shift+S |
| Open / New | ⌘O / ⌘N | Ctrl+O / Ctrl+N |
| Duplicate step | ⌘D | Ctrl+D |
| Delete step | ⌫ | Backspace / Delete |
| Run / Quit | ⌘R / ⌘Q | Ctrl+R / Ctrl+Q |
| Group / Ungroup | ⌘G / ⇧⌘G | Ctrl+G / Ctrl+Shift+G |
| Open group / go back out | Enter (or double-click) / Esc | Enter (or double-click) / Esc |
| Add to selection | Shift-click | Shift-click |

In a text or number box: Enter/Tab to confirm, Esc to cancel, ⌘V / Ctrl+V to paste, ⌘⌫ / Ctrl+Backspace to clear.

## Files

- Macros are saved as readable JSON. By default they go in `Documents/MacroFlow Macros/` in your home folder.
- Snipped images are saved in `Documents/MacroFlow Macros/images/`.
- `examples/hello.json` is a small sample you can open.

## Project layout

```
src/
  main.rs    app start-up, menu, window
  ui.rs      the editor: toolbar, palette, flowchart canvas, inspector
  model.rs   macro document (steps, connections, delays), saved as JSON
  engine.rs  background threads: playback, recording, position picking, snipping
  vision.rs  screen capture, area selection, OCR, window list, template matching
helpers/
  ocr.swift  macOS OCR helper (Apple Vision), compiled by build.rs
  ocr.ps1    Windows OCR helper (Windows.Media.Ocr), embedded into the app
build.rs     compiles the macOS OCR helper
.github/workflows/build.yml   builds + tests on macOS and Windows
  keys.rs    key names ↔ macOS key codes ↔ recorder/GPUI key names
```

Main libraries: `gpui` (UI), `enigo` (sending input), `device_query` (reading input while recording),
`xcap` (screenshots), `image` (image handling), `serde_json` (saving).

## Troubleshooting

**Windows**
- **`link.exe not found` / MSVC errors**: install Visual Studio Build Tools with "Desktop development with C++".
- **`fxc.exe` not found** (release builds): set `GPUI_FXC_PATH` (see Setup).
- **Clicks/keys don't reach an app**: that app is probably running as administrator. Run MacroFlow as administrator.
- **Select area shows an error / no box**: MacroFlow shows the selector's error in the status bar. Details are also logged to `%TEMP%\macroflow-select.log`.
- **Read text says no OCR language**: install a language pack that includes OCR (Settings → Time & language).

**macOS**

- **`unable to find libclang` / `linker cc not found`**: run `xcode-select --install`.
- **`xcrun: error: unable to find utility "metal"`**: make sure `Cargo.toml` still has `features = ["runtime_shaders"]` on `gpui`.
- **Compile errors inside GPUI**: GPUI changes a lot between versions. This project is pinned to `gpui = "=0.2.2"`. Don't change that line unless you mean to upgrade.
- **Nothing happens on playback / recording is empty**: check Accessibility (and Input Monitoring), then fully restart the terminal.
- **Find image never matches**: check Screen Recording permission, then use **Test now** to see the best match score. Lower the threshold (try 0.75–0.8) or snip a smaller, more distinctive area.
- **A macro won't stop**: press **Esc**. The stop check runs on its own thread every 25 ms.
- **"The text reader isn't built"**: the Swift OCR helper failed to compile. Look for `OCR helper:` warnings in the `cargo build` output, run `xcode-select --install`, then `cargo clean && cargo build --release`.
- **Read text reads the wrong thing / nothing**: draw the box tightly around just the text or number (a little space round it, no icons or labels), and make sure MacroFlow isn't covering it when the macro runs. **Read now** minimises MacroFlow, reads, and shows what each attempt saw when they disagree.
- **Window check never sees window titles** (only app names): turn on Screen Recording for your terminal and restart it.
- **MacroFlow stayed minimised after Select area**: click its icon in the Dock / taskbar.

## Limits / ideas for next steps

- On Windows, mouse playback is mapped to the primary display, so positions on other monitors may be off.
- Scroll-wheel events aren't captured by the recorder. Add a Scroll step instead.
- Possible additions: "If pixel colour" and "If key held" branches, variables/counters,
  a global start hotkey, zoom on the canvas, converting a recording into separate steps, and undo/redo.
