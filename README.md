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
| **Canvas** | **Ctrl + scroll** zooms in and out around the mouse (⌘ + scroll also works on a Mac); the **− 100% + Fit** box at the bottom right does the same, and clicking the % resets to 100%. **Right-click** a step for Edit settings, Rename (F2), Run from here, Group / Ungroup / Open group, Duplicate, Disconnect all and Delete; right-click several selected steps to group or delete them together; right-click empty canvas to **Add step here**, Select all, Zoom to fit, Reset zoom or go Full screen. (On a Mac, Control-click works as right-click.) Drag steps to move them. Drag from a step's **right-hand dot** to another step to connect them. Click the **×** on a line, or right-click a dot, to disconnect. Drag empty space or scroll to pan. |
| **Log tab** | Every step the last run went through, with timings, what Read text read, what Find image found, variable values and which way each branch went, ending with why it stopped. Click a line to jump to its step. If a step went wrong it also shows a screenshot of the screen at that moment (saved in `Documents/MacroFlow Macros/images/`). **Copy** copies the whole log. |
| **Status bar** | Shows what's happening and which step is running. The running step is outlined in yellow. When a run ends, it says **why** (reached an End step, a path with nothing connected, you pressed Esc/Stop, or a step went wrong), and the step responsible is outlined in **red** with a "stopped here" tag. If it ended on an unconnected output, that output's dot is red too. **Show step** jumps to it, even inside a group. ✕ clears the highlight. |

If a step is selected, new steps are placed after it and connected automatically.
If you close the window or quit with unsaved changes, MacroFlow asks whether to **Save**, **Don't Save** or **Cancel**.

**Esc stops a recording or a running macro**, even when MacroFlow isn't the active window.

### Step types

| Step | Outputs | Notes |
|---|---|---|
| **Start** | next | Every run begins here. |
| **Click** | next | Left / right / middle, single or double click, at a fixed spot, **randomly inside an area** (drag a box around it, then optionally give it a **shape**: rectangle, ellipse, diamond, triangle or **freeform** (any outline you click out yourself), **turned** to any angle, and **hollow** so only a band along its edge counts, e.g. a hollow diamond round the outside of a base. **Edit on screen…** opens the area over a screenshot so you can move, resize and turn it with handles and drag freeform corners about. A preview shows where clicks can land, and **Show on screen** flashes the green dot at 24 random spots), at the found image (+ offset), or wherever the pointer is. **Random offset (px)** moves each click up to that many pixels in a random direction, so it never clicks exactly the same place twice in a row. "Pick position on screen" captures the pointer position after 3 s. |
| **Drag** | next | Presses a mouse button at a start spot, moves smoothly to an end spot and lets go, for sliders, drag-and-drop or selecting text. **Pick start / end on screen** captures each spot after 3 s. You can set the button and how long the move takes. **Hold still before moving** keeps the mouse still for a while after pressing, before it moves, for apps that only pick something up after a still hold (otherwise they pan the view). **Tap instead of dragging** (taps above 0) taps that many times instead: evenly **along the line**, or **round a shape**, an area with the same shapes, turning, hollow band and on-screen editor as Click's random area. Round a hollow shape the taps go in order round its edge band (e.g. troops round the outside of a base). In a solid shape they're spread evenly over it. |
| **Move mouse** | next | Same targeting options as Click. |
| **Key press** | next | Any key plus ⌃⌥⇧⌘. Use "Capture key from keyboard" to set it by pressing the keys. |
| **Type text** | next | Types a string, optionally followed by Return. |
| **Wait** | next | Fixed wait plus optional random extra time. |
| **Scroll** | next | Scrolls up/down or left/right by a number of **wheel clicks**, sent one at a time like turning the wheel by hand (many apps and games count one big scroll as a single click). **Strength** makes each click go further if it still scrolls less than your mouse. **Test (scrolls in 3 s)** scrolls wherever you point the mouse, so you can tune it. |
| **Recording** | next | A recorded sequence of raw events. Each event has its own delay, which you can edit, nudge with −/+ (Shift-click for ×10), or delete. You can also halve or double all delays, set them all to one value, remove mouse moves, or change the playback speed. |
| **Find image** | **found** / **not found** | Looks for an image on every display until it appears or the timeout runs out. After it runs (or after **Test now**), the picture at the top shows **what it saw**: the search area (or the screen around the best match) with a green box round the match, or an amber box round the closest thing if it wasn't found. **Image to find** switches back to the picture it looks for. **Drag a box on screen** around what to find, snip the area around the pointer, or choose a PNG. **Look in → One area** limits the search to part of the screen you drag a box around: much faster, and it can't match the same picture somewhere else. "Test now" checks it straight away. You can make it click the image when found. Later Click / Move steps can target the found image. |
| **Window check** | **yes** / **no** | Checks whether the **current window** (the front window of the app you're using) or **any open window** has a title / app name that contains, equals or starts with some text. Optional match case, "keep checking for N ms" (to wait for a window to open or come to the front) and "bring it to the front if found". **Show open windows** lists what's open right now. Click one to fill in the text. |
| **Read text** | **match** / **no match** | Reads the text in a screen area using the OCR built into the system (Apple Vision on macOS, Windows OCR on Windows). Numbers drawn the way games often do (a white or coloured fill with a thick dark outline, on a busy background) are handled by first keeping only the letters' fill colour and throwing away the outline and background, so they read as plain black digits. After it runs (or after **Read now**), the picture at the top shows **what it actually read at that moment**, with **what the reader was given** (the cleaned-up version) underneath, with what it read underneath (green border = match, amber = no match). Switch between **Last run** and **When selected** to compare it with the area as it looked when you selected it. Each read cleans the image up (enlarges it, boosts contrast, flips light-on-dark text, adds a margin), reads three versions at once and takes the answer they agree on, re-reading if they don't. Number comparisons (and **Only read a number**) fix common misreads (O→0, l→1, S→5), join split digits and handle thousands separators. Click **Select area (drag a box)**: MacroFlow minimises and you drag a box around the text or number anywhere on your screen. Compare as **text** (contains, is exactly, starts with, matches regex, has any text, has changed) or as a **number** (=, ≠, >, ≥, <, ≤, using the first number in the area). "Read now" shows what it reads. "Keep reading for N ms" waits for the condition. A Type text step can type what was read with `{text}`. |
| **Set variable** | next | Stores a value in a named variable: a number (`0`), a sum (`gold + 1000`, `(a + b) / 2`, `count * 3`, `total % 10`), another variable, or text with `{name}` in it. Unset variables count as 0 in sums, so `count = count + 1` works straight away. Built-in variables: `text` (last Read text), `run` (which repeat this is), `found_x` / `found_y` (last found image). Use any variable as `{name}` in Type text and Pop-up steps. Read text can also **save what it reads into a variable**. Variables show up as purple **`# name` tags** wherever they're used: on the step cards and in text boxes. Typing the closing `}` of `{name}` turns it into a tag, which then moves and deletes as one piece. While you edit a box that takes variables, the variables you already have are listed under it, so a click puts one in. Read text's **Compare with** can use `{name}` too (e.g. `{gold_before}`). The **Macro** tab lists every variable. An **amber** tag is a variable that no step sets, which is usually a typo. |
| **If** | **yes** / **no** | Compares two values (numbers, variables, sums or text) with =, ≠, >, ≥, <, ≤, contains or doesn't contain, e.g. `gold ≥ gold_before + 1000`. Numbers compare as numbers; text ignores case. |
| **Pixel colour** | **match** / **no match** | Checks the colour of one spot, e.g. a health bar or a button turning green. **Pick spot & colour** samples it after 3 s; tolerance allows small differences; "Keep checking for" waits for the colour. Much faster than Find image. |
| **Pop-up** | next | Shows a small card at the top of the screen (top centre or top right) with a title, message and style (Info / Success / Warning / Error), optionally with a sound. It never takes focus, so it doesn't interrupt what the macro is typing or clicking. It fades out after a set time, or with **Pause until I click it** the macro waits for you to click it. `{text}` shows what the last Read text step read. **Preview pop-up** shows it without running the macro. |
| **Group** | next | A collapsed run of steps that shows as one step (a little stack of cards). Double-click it to open it. See **Groups** below. |
| **Loop** | **repeat** / **done** | Runs the "repeat" branch N times (0 = forever), then follows "done". Connect the last step of the loop back into the Loop step. |
| **End** | none | Stops the macro. A step with nothing connected also stops it. |

Every step also has a **Note** and a **Delay before** field.

### Start screen, templates and autosave

When MacroFlow opens (and after **New**) a start screen shows over the canvas with:

- **Blank macro**, **Open a file…** and a few **templates**: *Click over and over*, *Find and click an image*, *Watch a number* and *Wait for loading to finish*. Template steps that still need setting up (an image to snip, an area to select) show a ⚠.
- **Recent** macros you've opened or saved (kept in `Documents/MacroFlow Macros/recent.json`).
- **Unsaved work** from last time. While a macro has unsaved changes it's copied to `Documents/MacroFlow Macros/autosave.json` every few seconds. If MacroFlow crashes or the computer turns off, **Restore** brings it back. Saving, or choosing **Don't Save** when you close, removes the copy.

It goes away as soon as you change anything, or press ×.

### Adding steps quickly

- **⌘ / Ctrl + K** opens **quick add**: type part of a step's name ("read", "click", "gone"…), use ↑ ↓ and press **Enter**. The new step is added after the selected step, like clicking it in the Steps tab.
- **Drag from an output dot and let go on empty canvas** to pick a step to add right there, already connected to that output.

### Checking a macro, and seeing what happened

- **⚠ badges.** A step that looks wrong gets a ⚠ in its corner: **red** if the run would certainly go wrong there (no image to find, no area to read, no key chosen, nothing connected to Start, an invalid variable name), **amber** if it's probably a mistake (an output that isn't connected, a step nothing leads to, a variable no step sets). A group shows a ⚠ if a step inside it has one. Select the step to read what's wrong in the Edit tab.
- **Check before running.** If any step has a red problem, pressing Run lists them first. Click one to go to it, or choose **Run anyway**. With F8 from another app, a pop-up tells you why it didn't start.
- **Results on the cards.** During and after a run, each step that ran has a small tag under it saying what it did: `read “18570671” → match`, `found 94%`, `best 61% → not found`, `= 1250`, `3 of 10`, `12 ≥ 10 → yes`, `gone after 2.3s`, or ✓. Green means it took its first output, amber the other one. **×5** means it ran five times. Steps with no tag didn't run.
- **Variables panel.** While a macro runs (and at breakpoints), a panel at the bottom left of the canvas shows every variable and its current value. It stays showing the final values after the run. × hides it until the next run.

### Waiting until something is gone

The **Wait until gone** step (in the Steps tab, or a Find image step with **Wait until it's gone** turned on) waits for an image to *disappear*, such as a loading screen or a "Connecting…" box. It follows **gone** as soon as the image isn't on screen any more, or **still there** if it's still showing when the time runs out. **Read text** has the same option: **Wait until it stops matching** keeps reading until the comparison stops being true (e.g. until the area no longer contains "Loading").

### Disabling steps and groups

Turn a step or a whole group off without deleting it or rewiring anything: right-click it → **Disable step (skip it)** / **Disable group**, select it and press **⌘E / Ctrl+E**, or use **⊘ Disabled** in its Edit tab. Select several steps to disable them all at once; the same again turns them back on.

- When a run reaches a disabled step it **skips it and carries on with whatever it's connected to**. A disabled group skips everything inside it.
- A step with more than one output (Find image, If, Read text…) carries on along its **first output that's connected**, normally the first one ("found", "yes", "match").
- Disabled steps look faded with a crossed-out name and **⊘ OFF**. Inside a disabled group, everything is faded and a note says the group is disabled.
- The Log shows "⊘ Skipped …" and the card gets a "⊘ skipped" tag. Disabled steps never get ⚠ warnings, since they don't run.
- It's saved with the macro (and undo works), and older macro files open as before.

### Debugging: step by step and breakpoints

- **⏭ Step** in the title bar (or **F10**) starts the macro paused before its first step. Each further F10 / **Step** runs one step and pauses again. The step about to run is outlined in yellow, and the status bar and Log say which it is. **Resume** (F9) runs the rest normally.
- **Breakpoints:** right-click a step → **Add breakpoint**, or tick **Breakpoint** in its Edit tab. A red dot appears on the step, and the macro pauses just before it runs. From there you can Step or Resume. Breakpoints are saved with the macro; older macros simply have none.

### Playing a run back (Log tab)

Every click, mouse move, drag, scroll, key press and bit of typing gets its own line in the **Log** tab (◉ Left click at (512, 300), ⇢ drag, ⇵ scroll, ⌨ key, T typed…), and MacroFlow keeps a picture of the screen at that moment with the action drawn on: a **green dot** where it clicked, a ring for a move, a line and ring for a drag.

- **Click any line** to see it. A **Playback** card opens at the bottom right of the canvas with the picture and what happened, the step it belongs to is outlined on the canvas (opening its group if needed), and the green dot also flashes at that spot on your real screen (turn that off with **Show on screen**).
- **▶ Play back** (in the Log tab, or on the card) goes through the whole run again at roughly the speed it ran, highlighting each step in turn. **1× / 2× / 4×** changes the speed.
- Keys while the card is open: **Space** plays / pauses, **← →** go a line back / forward, **Esc** closes it.

The pictures are taken in the background so they don't slow the macro down. Up to 400 are kept per run (in `Documents/MacroFlow Macros/images/last-run/playback/`, cleared at the start of each run and when MacroFlow opens). If the macro is very fast, an action may have no picture, but it's always listed.

### Editing an area on screen

**Edit on screen…** (in a Click / Move mouse step's random area, or a Drag step tapping round a shape) hides MacroFlow, takes a screenshot of that display and opens it full size with the area drawn on top. The green dots show where clicks can land, or exactly where the taps will go.

- **Drag inside** the shape to move it. The **squares** on the corners and sides resize it, and the **round handle** above it turns it (hold **Shift** to snap to 15°; **← →** nudge it a degree). **Drag outside** the box to draw a new box.
- The toolbar switches the **shape**, sets the **hollow edge** band (− / +, Shift for bigger steps) and **Straighten**s it.
- **Freeform:** drag a corner dot to move it, **click outside the outline** to add a corner on the nearest side, and **right-click** a dot to remove it. **Draw a new outline** clears it, then you click each corner in turn and click the first corner again (or press Enter) to finish.
- **Save** (Enter) keeps it, **Cancel** (Esc) leaves the step as it was. The area still scales with the screen size and follows a window like any other position.

### Seeing where it clicks

While a macro runs, a **green dot** appears where it's about to click, so you can see where the mouse is heading before it gets there. When it clicks, the dot ripples and disappears. A plain **Move mouse** shows a green ring instead, and a **Drag** shows a ring where it will let go.

The dot can't get in the way:
- Clicks and the mouse go straight through it, and it never takes focus.
- It's hidden from screen capture (macOS: the window is excluded from sharing; Windows 10 2004+: excluded from capture). On top of that, MacroFlow takes it down before **every** screenshot it takes, so Find image, Read text, Pixel colour and failure screenshots never see it.

Turn it off with **Show a green dot where it clicks** in the **Macro** tab.

### Human-like mouse movement and the failsafe

- **Human-like mouse movement** (Macro tab) makes the mouse glide to every spot along a slightly curved path that starts slow, speeds up and slows down, different every time. Longer moves take longer. **Mouse speed** adjusts it. Recordings still play back exactly as recorded. It's off for older macros until you turn it on.
- **Failsafe** (Macro tab, on by default): throwing the mouse into any outer corner of the screen stops the macro instantly. Corners where two monitors meet don't count, and nor does the macro itself moving there. The status bar says "Stopped by the failsafe".

### Hotkeys, pausing and alerts

- **F8** starts the macro from any app, and stops it while it runs. **F10** runs one step (see Debugging above). **F9** pauses and resumes (the ❚❚ Pause button in the title bar does the same). **Esc** still stops.
- If a run ends **unexpectedly** (a step went wrong, or it reached a path with nothing connected), a **red pop-up** appears at the top of the screen saying why, even when MacroFlow is minimised. Reaching an End step or pressing Stop doesn't count. Turn it off in the **Macro** tab.
- Turn on **Green pop-up when it finishes** (Macro tab → When it stops) to get a pop-up when it reaches an End step or has done every repeat. Pressing Stop / Esc never shows one.
- When it pauses at a **breakpoint**, a pop-up says which step it stopped before and which keys to press, so you notice even if MacroFlow is hidden. Turn it off with **Pop-up when it pauses at a breakpoint**. Step-by-step mode doesn't show one for every step.
- If MacroFlow itself crashes during a run, that counts as a step going wrong, so the red pop-up shows too.
- If MacroFlow itself crashes, a red "MacroFlow crashed" pop-up appears with the reason.

### Working on other screens and computers

In the **Macro** tab, **Scale positions to the screen size** saves every position as a *place on the screen* instead of exact pixels. Moving the macro to a bigger or smaller monitor, or another computer, puts each click, read area, search area, colour check and recording in the same spot. Find image pictures are resized to match too. It's **on for new macros**. Older macros keep exact pixel positions until you switch it on; switching it on doesn't move anything on your current screen. Positions are measured against the main display. If the macro also follows a window, they're measured against that window instead, so they scale with the window's size.

### Following a window

In the **Macro** tab, **Follow a window…** makes every saved position (clicks, click areas, read areas, search areas, colour checks, recordings) relative to the window you pick. Move that window, or use a different monitor, and the macro still clicks the right places. The window is found by app name (and title when it matches). **Use screen positions** turns it off without moving anything.

### Undo, copy and paste

- **Ctrl+Z** undo, **Ctrl+Y** / **Ctrl+Shift+Z** redo (⌘ on a Mac), or the ↶ ↷ buttons in the title bar. A whole drag is one undo.
- **Ctrl+C / Ctrl+X / Ctrl+V** copy, cut and paste the selected steps and the connections between them. They go on the system clipboard, so you can paste into another macro. Right-click empty canvas → **Paste here** pastes where you clicked.

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
| Rename step | F2 | F2 |
| Zoom in / out / 100% / fit | ⌘= / ⌘− / ⌘0 / ⌘1 (or ⌘ + scroll) | Ctrl+= / Ctrl+− / Ctrl+0 / Ctrl+1 (or Ctrl + scroll) |
| Full screen | ⌃⌘F (or the green button) | F11 |
| Select all steps | ⌘A | Ctrl+A |
| Open group / go back out | Enter (or double-click) / Esc | Enter (or double-click) / Esc |
| Add to selection | Shift-click | Shift-click |

In a text or number box: click to put the cursor there, drag (or Shift-click) to select, double-click to select everything. **Ctrl+A** (⌘A) selects all, **Ctrl+C / X / V** (⌘C / X / V) copy, cut and paste, **← →** move, **Shift + ← →** select, **Ctrl + ← →** (⌥ ← → on a Mac) jump a word, **Home / End** (⌘ ← → on a Mac) go to the start / end, **Ctrl+Backspace / Delete** (⌥⌫) delete a word. **Enter / Tab** confirms, **Esc** cancels.

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
  marker.rs  the green dot showing where a running macro clicks
helpers/
  notify.swift macOS pop-up helper, compiled by build.rs
  notify.ps1   Windows pop-up helper (WinForms), embedded into the app
  marker.swift macOS green click-marker dot, compiled by build.rs
  marker.ps1   Windows green click-marker dot (WinForms), embedded into the app
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
- **Pop-up doesn't appear**: errors are logged to `%TEMP%\macroflow-notify.log`. The very first pop-up after starting MacroFlow can take a second to appear while Windows prepares it.
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
