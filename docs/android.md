# Android

The engine as a native Android app (engine addition): the same crate built as a `cdylib` for Android, started by a small
`NativeActivity` subclass, drawn by wgpu (Vulkan, GLES as the fallback) at full speed, offline but for the NETWORK menu. The game data is
not in the APK: at the first start the engine's own setup screen (the same as on desktop, `src/app/setup/`) asks for
the player's converted game data, picked with the system's folder picker, and copies it once into the app's private
storage. Touch controls in the PS1 layout (a d-pad by default, an analog-style stick as an
option) drive player 1 with the same pad bits as the keyboard and gamepads; the simulation is not touched
(the NETWORK menu's row on the title is `SIM_VERSION` 8's change, [`network_menu.md`](network_menu.md)).

## Build pipeline

`android/build.sh` (needs the SDK, the NDK, `cargo-ndk` and a JDK; downloads nothing):

1. `cargo ndk -t arm64-v8a --platform 26 build --release --lib` builds `libopen_kumite_beast.so` (the crate's
   `cdylib`; the entry is `android_main`, `src/lib.rs`). `-t x86_64` too with `ABIS="arm64-v8a x86_64"`.
2. `javac --release 11` compiles `android/java/.../OkbActivity.java` against the platform's `android.jar`;
   `d8 --min-api 26` turns it into `classes.dex`.
3. `aapt2 compile` compiles `android/res/` (the launcher icon only) and `aapt2 link` packages it with
   `android/AndroidManifest.xml` into an APK;
   the dex and `lib/<abi>/*.so` are added with `zip` (the libraries stored, not compressed),
   `zipalign -p 4` page-aligns them (the manifest's `extractNativeLibs` unpacks them at install, and `build.rs`
   links the library with `-z max-page-size=16384` for 16 KiB-page devices with any NDK), `apksigner` signs with the debug keystore (`~/.android/debug.keystore`, or one
   made in `android/build/` when there is none).
4. Output: `android/build/okbeast.apk`; `adb install -r android/build/okbeast.apk`.

Why not Gradle: the Android Gradle plugin and Gradle itself would be downloads and a second build system around
one Java file; the SDK's build tools do the four steps directly and the script stays readable. Why not `cargo-apk`
/ `xbuild`: `cargo-apk` is deprecated and neither is installed; both build a bare `NativeActivity` APK, and this one
needs a Java subclass (the folder picker's activity result, the input hand-over, immersive mode).

## Versions and ABIs

- **minSdk 26 (Android 8.0)**: AAudio for cpal / oboe, `AudioFocusRequest`, `EXTRA_INITIAL_URI` for the picker,
  Vulkan 1.0 on most devices of that age; older devices would need OpenSL ES and lose little else, but the GPU
  drivers of that era are the bigger risk. **targetSdk 35.**
- **arm64-v8a** (every current phone, and the emulator image on an Apple-silicon Mac); x86_64 on request for x86
  emulators. 32-bit ABIs are not built.
- Landscape only (`sensorLandscape`); the activity handles every change `configChanges` can list itself (all 18
  public flags of API 35: orientation, screen size and smallest size, screen layout, keyboard and keyboard hidden,
  navigation, UI mode (dark mode, car / desk dock), density, language and layout direction, font scale and font
  weight, grammatical gender, colour mode, touchscreen and SIM (`mcc` / `mnc`)), so a rotation, a dark-mode switch,
  a controller connecting, a language or font-size change or a SIM swap does not recreate it: `onConfigurationChanged`
  only hides the bars again and asks for the insets (NativeActivity passes the change on; winit resizes the surface
  when the size changed). What no flag covers (a theme overlay's assets, e.g. a wallpaper-colour change on some
  launchers) still recreates it: the process then restarts (Lifecycle, below), the rare fallback.

## The activity (`android/java/com/openkumitebeast/game/OkbActivity.java`)

A `NativeActivity` subclass, `android-activity`'s native-activity backend (winit's `android-native-activity`
feature) runs `android_main` on its own thread. The Java side does what needs the framework:

- **Game data.** The engine checks its game data folder (`files/openkumitebeast/game_data`, `platform::game_data`) at
  every start as on desktop (`assets::manifest`: the format, every listed file's size, the engine's required set);
  missing or unusable data (or a start that cannot read it) opens the setup screen (`src/app/setup/`, drawn by the
  engine itself: the dark red glow and claw marks of the old Java backdrop are its backdrop). CHOOSE FOLDER... opens
  the system's folder picker (`ACTION_OPEN_DOCUMENT_TREE`, starting in Download; `OkbActivity.pickFolder`, the answer
  through `nativePicked`); the picked tree is listed by the activity (`listTree`: hidden files and `play/` left, a
  name with `/`, `\`, a tab, a newline or NUL in it, more than 16 levels or 100 000 entries refused) and the
  engine's own job (`app/setup/job.rs`, on a thread) takes the top or a `br2_export` folder in it, checks it, checks
  the free space (`freeSpace`), copies the files the engine reads into `game_data.part/` (each opened as a file
  descriptor, `openDocument` → `ParcelFileDescriptor.detachFd`), reads every copied file back to compare its CRC-32,
  and swaps the folder in; the game then starts without a restart. A cancel or a failure leaves the old data and no
  `.part`; the app sent to the background meanwhile keeps copying. Old installs are not read any more: an old export
  picked in the setup is refused with "convert the game data again"; the activity's own copy of earlier versions
  (`files/br2_export`) is ignored and left on disk.
- **Convert disc image** (feature `converter`): CHOOSE FILE... opens the system's document picker
  (`ACTION_OPEN_DOCUMENT`, `pickFile`); the picked image (a raw `.bin`, `.chd` or `.iso`: a `.cue`'s `.bin` cannot be
  reached through a single picked document) is opened as a detached file descriptor (`openUri`) and read at random
  places by the converter (`ok_br2_converter`, up to 4 threads) into `game_data.part`, then checked and swapped in as a
  copy. A descriptor that cannot seek (a streaming provider) is refused with "copy it to the device first". On the
  `br2_test` emulator the conversion takes 17-33 s (33 s with the screen drawing, 17 s with the app in the background,
  where it goes on). A picked `.cue` (told by the document's name) is refused with what to choose instead. The second
  disc (ADD BLOODY ROAR 1 DISC) is picked the same way; its CD audio track (Track 2) comes only with a `.chd`.
- **Change game data** (OPTIONS → CHANGE GAME DATA, simulation v9): the same setup screen over the waiting session
  (BACK TO THE GAME closes it); a new install restarts the activity (`restartApp`: the loaded data would be stale).
  Off in netplay.
- **A failed start.** Only a start that cannot draw at all (no GPU) or a match that does not load ends in the
  activity's dialog (`showFatal`, QUIT: `finish`); the native side goes idle meanwhile (ending the event loop from
  here would race the activity's UI thread). Unreadable game data goes to the setup screen instead.
- **Debug hooks.** The desktop's OKB_* variables come from the launch intent's `okb_env` extra (`KEY=value;…`; read
  only in a debuggable build, as `build.sh` makes, since the launcher activity is exported):
  `adb shell am start -n com.openkumitebeast.game/.OkbActivity --es okb_env 'OKB_SETUP_FROM=/sdcard/Android/data/com.openkumitebeast.game/files/okb_export'`
  (the app's own external folder, readable without a permission) drops that folder onto the setup screen.
- **Input.** The activity gives the input queue back to the view hierarchy (`takeInputQueue(null)`) and forwards
  every touch, key and joystick event to Rust (`nativeInput`, a queue drained once per drawn frame). winit's
  Android backend turns joystick motion into touch moves and drops the hat and trigger axes, so the engine reads them
  itself; this way gamepads (Bluetooth / USB, `SOURCE_GAMEPAD` / `SOURCE_JOYSTICK`), keyboards and touch all arrive
  the same way, each key and axes event with its `InputDevice` id (a device that goes away is reported too, through
  an `InputDeviceListener`). Volume keys go back to the system. The queue is bounded (4096 events): when it is full
  the oldest touch, key or axes event makes room, never a device going away nor the insets.
- **Players.** As on the desktop ([gamepads.md](gamepads.md)): the keyboard, the touch controls and the first
  gamepad used are player 1's, the second distinct gamepad (another device id) is player 2's, the third and fourth
  players 3 and 4's, read only by the team battle
  (`app::devices::PadSlots`, shared with the desktop); a fifth is not read. Pads 3 / 4 have no press latch (below):
  they are read as held, so a press shorter than a frame can be missed. A gamepad is an event from a gamepad or
  joystick source or from a device with sticks (a pad's d-pad may arrive as plain DPAD keys). Each keeps its player
  until it goes away; its player is then kept for it, and the next new gamepad used takes it (Android gives a pad
  that comes back a new device id, and all pads count as one model here); the other keeps its own. Only player 1's
  devices hide the touch controls. Untested on hardware with two real pads; on the emulator two virtual gamepads
  (`uinput`, below) show it.
- **Icon.** An original adaptive icon (`android/res/`, written by `tests/android_icon.rs`:
  `OKB_WRITE_ICON=1 cargo test --test android_icon` after a change there; a plain `cargo test` checks the committed
  files): "OKB" (OpenKumiteBeast) as stroked vector letters (no font, no game artwork) on a dark red background, with a monochrome
  layer for themed icons.
- **Window.** Full screen at all times, edge to edge: the status and navigation bars hidden (API 30+
  `WindowInsetsController.hide(systemBars())` with `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE`, API 26–29 the
  `SYSTEM_UI_FLAG_IMMERSIVE_STICKY` flags): a swipe from an edge shows them over the picture and they hide again by
  themselves. Applied on create, resume, focus back (the folder picker, recents, the shade) and every configuration
  change (a rotation to the other landscape). The window is drawn under the display cutout too
  (`layoutInDisplayCutoutMode` ALWAYS on API 30+, SHORT_EDGES on 28–29; in landscape the default mode would leave a
  black bar on the cutout's side; API 26–27 have no cutout API). The activity's dialogs (game data, copy progress,
  copy errors, a failed start) are shown the same way (`show`: the dialog's window starts unfocusable, is made
  immersive, then takes input), so the bars stay hidden behind them. The picture fills the whole screen; the touch
  controls stay inside the cutout's safe insets (sent to Rust, `Input::Insets`). Keep-screen-on while the activity is
  in front (`FLAG_KEEP_SCREEN_ON`, cleared by the system in the background). The fight HUD keeps inside the same
  insets (Screen mode, below). Not taken into account: the screen's rounded corners (`WindowInsets.getRoundedCorner`).
- **Screen mode.** WIDE (GRAPHICS → SCREEN MODE FULL) is the default on every target, Android included: a first
  start, or a `prefs.json` without the aspect, starts in WIDE; CLASSIC chosen in OPTIONS is saved and kept. On a
  20:9 phone (2400×1080, 2.22:1) WIDE fills the whole screen: the 3D view widens up to 21:9 (`MAX_ASPECT_3D`, only a
  wider screen gets side bands), and fixed 4:3 pictures (title, movies, story pictures) get the blurred ambient fill
  beside them, as on desktop. The fight HUD goes out to the screen's edges (on desktop and the web its edge anchors
  stop at 16:9, `MAX_ASPECT_HUD`): its left-side items (player 1's life bar, gauges, name, round marks) are anchored
  to the left edge, the right-side ones to the right edge, the centre ones (the timer) stay centred, all inside the
  cutout's safe insets (`Viewport::with_insets`, given by `Renderer::set_hud_insets` every frame from the insets the
  activity sends; as far as the 21:9 band on a wider screen). So with the punch hole on one side that side's HUD
  sits the cutout's inset further in; a rotation to the other landscape sends the new insets. Top and bottom keep
  the 16:9 cap of the desktop (a window taller than that, in multi-window). CLASSIC is unchanged.
- **GRAPHICS rows.** OPTIONS → GRAPHICS has no WINDOW, WINDOW SIZE, VSYNC or FRAME CAP on Android (the app is
  always full screen and paced by the display, whatever those values say: `app::requests::presented` presents with
  vsync on and no frame cap there, so a value saved by an older build or set in a desktop host's netplay rows has no
  effect): SCREEN MODE, RENDER QUALITY, HUD SIZE, PERF OVERLAY, DEFAULT, EXIT;
  the cursor skips the hidden rows both ways, and DEFAULT leaves their values as they are in `prefs.json`. The rows
  are part of the session (`SessionStart::android`, set by the app on Android, recorded in a replay's header and in
  the netplay start blob as `browser` is, `GameState::android`, `Menu::android`): an Android replay plays the same
  rows on the desktop, a desktop one the desktop rows on Android. Older files and every other build read false: no
  `SIM_VERSION` change; `STATE_VERSION` 7.
- **Audio focus.** Requested in `onResume` (`AUDIOFOCUS_GAIN`, game usage), abandoned in `onPause`.

## Lifecycle

| Android | winit | the app |
|---|---|---|
| window created (`onNativeWindowCreated`) | `resumed` | first time: the window, then the startup load (or the setup screen without usable data); later: the GPU surface is created again for the new native window (`Renderer::resume_surface`) |
| focus lost (shade, dialogs, recents) | `Focused(false)` | inputs released, the session stops stepping, the audio stream is closed |
| focus back | `Focused(true)` | the frame clock restarts (no catch-up burst), the audio stream is opened again |
| window destroyed (home, screen off) | `suspended` | the surface is dropped (`Renderer::suspend`); nothing is drawn or stepped until `resumed` |
| back key / gesture | key `KEYCODE_BACK` | the TOUCH CONTROLS page or the replay list closes; in a fight START (the pause menu); elsewhere △ (back) |
| activity destroyed (Quit, destroyed by the system in the background, or recreated for a change `configChanges` cannot cover, e.g. a theme overlay) | — | the process exits in `onDestroy` (NativeActivity's destroy would wait for `android_main` to return, and a winit event loop is made once per process); only a recreation (`isChangingConfigurations`) starts the activity again in a fresh process, one destroyed in the background stays closed until the player starts it |
| process killed in the background | — | the next start is a cold start at the boot logos; the card and preferences were saved when they changed |

The session is frozen, not paused in the game's sense, while the app is in the background: no in-game input is
made up, so replays and the state stay exactly what the player did.

## Files in app storage

`Context.getFilesDir()` (`/data/data/com.openkumitebeast.game/files`, private, removed with the app):

- `openkumitebeast/`: what `platform::storage` keeps in the config directory on desktop: `game_data/` (the game data,
  with `game_data.part/` while an install runs), `save.json` (the memory card),
  `prefs.json` (GRAPHICS), `touch.json` (the touch controls, below), `net.json` (the NETWORK menu), `replays/`, crash and support reports.

Earlier builds were the package `com.br2port.game`; the new package id installs as a separate app, so the old app's
`files/br2_engine/` (its save, preferences, replays and game data) is not carried over and the old app can be
uninstalled once it is no longer needed.

## Touch controls (`src/app/touch.rs`)

- In fights: placed between the fight HUD's rows in CLASSIC and WIDE alike (the core picture spans the window's
  height): the d-pad and the face buttons end above 84 % of the height (the beast gauges start at 87.5 %), L1 / R1
  start below 20 % (the names end at 19.6 %) for every SIZE on 20:9, 16:9 and 4:3 screens, and all within the
  cutout's safe insets (`Layout::new`).
- Outside fights (the front end, the replay list, the NETWORK menu): clear of every screen's text and pictures
  (`Layout::front`). The free places were measured on the title, the mode select, the arcade, survival and VERSUS
  selects, every OPTIONS page, CUSTOM, the replay list, the NETWORK menu with its picker and the story pages, in WIDE
  and CLASSIC (the VS screen, the memory card dialogs and the name entry from their code), and are kept in the core's
  512 × 480 units (`FE_LEFT` / `FE_RIGHT`): the bottom corners below the VERSUS select's names (y 285; WIDE puts
  them beside the core), left of the arcade select's beast emblem, the list cursors and the story box's portrait,
  right of the replay list's arrows and the story box's arrow, and on the bottom line (below 440 / 450) beside the
  selects' portraits and the hint (x 64 / 180 and 345). The d-pad and the face buttons sit in those corners, each as
  large as the SIZE asks and the corner allows; L1 / OPT and START / R1 go beside the core from y 80 to 228 (only
  beast art and backdrops there, `FE_SIDE`) in a row or a column, both sides alike, else on the bottom line beside
  the d-pad and the face buttons. The list menus' main-game logo (bottom right) is decor and may be covered. So a 20:9
  phone keeps the SIZE's d-pad (up to 0.2 of the height, 0.16 with the emulator's cutout) with the small buttons
  beside the core; a 16:9 one has the sides only (0.11 of the height, 0.05 with a cutout).
- Only at a usable size (`FRONT_MIN`: the d-pad's radius at least 0.045 of the height, a face button's 0.025, which
  20:9 and 16:9 phones pass at every SIZE with or without a cutout). Otherwise (a 4:3 tablet, which has no room
  beside the core, or anything as narrow) the front end keeps the fight's layout: **the controls overlap the menus,
  see-through**. Only OPT moves, to the bottom-left corner below the menus (below 450, left of the hint), shrunk as
  far as half its size if needed; where even that does not fit it stays beside L1. On a near-square or portrait
  window (a foldable, split screen), where the fight's boxes crowd, OPT goes above L1 so it never lies on START.
- PS1 layout: the d-pad (default) or a floating stick on the left; on the right □ △ ○ × as on the pad; L1 above the
  d-pad, R1 above the face buttons, a small START beside R1 (and OPT, the TOUCH CONTROLS page, beside L1 outside
  fights and netplay; outside fights placed as above). Sizes follow the window height (and the cutout insets);
  multitouch: a held direction and any buttons at once, a finger between two face buttons presses both, sliding
  from one to another follows. A finger still on the d-pad when a fight starts or ends keeps its direction (read
  from where the d-pad was at the touch); the next touch uses the d-pad's new place.
- The bits are OR-ed into player 1's pad exactly as the keyboard's (`App::pads`), with the same latch for presses
  shorter than a frame: the simulation sees nothing new.
- Drawn after the frame in window pixels (`Renderer::screen`, a last pass over the whole window), so in CLASSIC
  4:3 the controls sit in the black bars and in WIDE over the picture.
- Hidden after a key or gamepad event, shown again on the next touch.
- **TOUCH CONTROLS page** (OPT): DIRECTION D-PAD / STICK, SIZE, OPACITY, EXIT (the game data is changed from
  OPTIONS → CHANGE GAME DATA). It is an app page
  like the replay list (the session waits while it is up), saved to `touch.json`: these are presentation
  preferences. They are deliberately not in OPTIONS → GRAPHICS, whose `Prefs` are part of the front end's
  serialised state and of every replay and netplay handshake.

## Netplay

The mode menu's NETWORK ([`network_menu.md`](network_menu.md)), as on the desktop: HOST / JOIN by room code (the
library is built with `netplay-webrtc`) or by direct IP (UDP), the code or address typed on the menu's on-screen
picker with the touch d-pad and buttons or a gamepad (no system keyboard). The APK asks for `INTERNET` (a normal
permission: granted at install, no prompt). Cross-play with the desktop and the browser: the same build's protocol
on every target. The background rule above stays: nothing is stepped or sent while the app is not in front, so
leaving it (home, the shade) for longer than the timeout (10 s) ends the session on both sides, each back on its
NETWORK menu with the message; a shorter absence only stalls the other player meanwhile.

## Testing

On an emulator (`br2_test`, arm64 `google_apis` android-35, landscape) with the data pushed to
`/sdcard/Download/br2_export` (`adb push`), driven by `adb shell input tap / swipe / keyevent` and checked with
`adb exec-out screencap -p`. The bars: `adb shell dumpsys window | grep -E "type=(statusBars|navigationBars) frame"`
(`visible=false` when hidden), the cutout mode in the window's `mAttrs` (`layoutInDisplayCutoutMode=always`); the
image's punch-hole cutout is on a long side in landscape, the other landscape by `adb emu sensor set acceleration
-9.81:0:0` (`9.81:0:0` back; `cmd window user-rotation` does not turn a `sensorLandscape` activity). Not tried on a
real phone: other cutout shapes, gesture against three-button navigation, OEM immersive variants. The desktop and web builds are unchanged (the Android code is behind
`cfg(target_os = "android")`; `app/touch.rs` builds and is unit-tested everywhere but is only switched on there).
