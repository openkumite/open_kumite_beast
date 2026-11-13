# Gamepads

Any gamepad works on desktop (macOS, Windows, Linux) and in the browser; on Android the system maps them
([android.md](android.md)). Gamepads are read outside the deterministic session: their buttons become the same raw PS1
pad bits as the keyboard's (`App::pads`), so nothing here touches `SIM_VERSION` / `STATE_VERSION`.

Code: `app/gamepads.rs` (`Pads`: the devices, players, rumble, the log), `app/pad_layout.rs` (the pure part: raw
codes → bits, the browser's layouts, the mapping file's check), `app/devices.rs` (`PadSlots`, shared with Android).

## Players

- The keyboard (W A S D …) is always player 1's, the arrows and numpad player 2's ([debug_hooks.md](debug_hooks.md)).
- The first gamepad **used** (a button pressed) becomes player 1's, the second distinct one player 2's, the third and
  fourth players 3 and 4's (only the team battle's screens and battles read pads 3 / 4). A fifth one is not read while four are connected. Moving a stick
  alone does not take a player (some devices report axes without being touched).
- A pad keeps its player while connected. When it goes away its player stays reserved for it: the same pad coming
  back gets it again (on Linux and Windows gilrs gives it its old id; on macOS a new id, then the reserved player of a
  pad of the same model, GUID and name, is given back on its first press).
- A new pad pressing a button takes players 1 / 2 before 3 / 4 (the team battle's): of those, a player never taken
  first, then one whose pad went away. So when player 1's pad dies and another one is picked up, that one becomes
  player 1 (if player 2 still has theirs), not player 3.
- Rumble goes to each player's pad (desktop; browsers have none here).

## How the buttons are read

| the pad | layout |
|---|---|
| known to the SDL database bundled with gilrs (PS3/4/5, Xbox 360/One/Series, Switch Pro, Logitech F310/F710, 8BitDo …) | its mapping: × = bottom face button, ○ right, □ left, △ top, L1/R1 shoulders, L2/R2 triggers, SELECT = Back/Share/Create, START = Start/Options |
| a line for it in `gamecontrollerdb.txt` (below) or in `SDL_GAMECONTROLLERCONFIG` | that mapping |
| Windows: an Xbox-compatible pad without a mapping | the system's Xbox layout, as above |
| no mapping (any other HID pad) | **default layout** by raw button number: 1 □, 2 ×, 3 ○, 4 △, 5 L1, 6 R1, 7 L2, 8 R2, 9 SELECT, 10 START (the DualShock 4 / DualSense order, used by most generic USB pads); the hat or d-pad and the left stick move |
| Linux, no mapping, the kernel names the buttons (BTN_SOUTH …) | those names, as SDL does; a joystick-style device (BTN_TRIGGER …) the default layout |
| browser, "standard" mapping (Chrome, Edge, Safari; Firefox for known pads) | the W3C standard layout |
| browser, no standard mapping (e.g. Firefox on macOS with a DualSense) | the default layout by button index + 1; a hat switch reported as one axis (Chrome's encoding: -1 up … +1 up-left, centred outside -1..1) moves the d-pad |

A device without a mapping that has fewer than 4 buttons (a 3D mouse, pedals) is not read (macOS, Windows; Linux's
backend only opens joysticks and gamepads). A pad whose buttons come out wrong needs a mapping line.

The web build does not use gilrs: its web backend does not tell whether a pad has the standard mapping (it reports
every pad's buttons by the standard order) and cuts a hat switch's axis to -1..1, where "centred" lies outside. The
app reads `navigator.getGamepads()` itself (`gamepads::Pads`, wasm). If Firefox reports a hat as four extra buttons
instead of an axis, those are past button 10 and the d-pad does not move; use the left stick, or Chrome.

## Your own mappings

gilrs reads SDL2 mapping lines, one pad per line:

```
030000004c050000e60c000000010000,PS5 Controller,a:b1,b:b2,x:b0,y:b3,back:b8,start:b9,leftshoulder:b4,rightshoulder:b5,lefttrigger:a3,righttrigger:a4,dpup:h0.1,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,leftx:a0,lefty:a1,platform:Mac OS X,
```

The GUID (32 hex digits, `xinput` for Windows' Xbox pads), a name, then `key:value` with SDL's names (`a` bottom face
button = ×, `b` right = ○, `x` left = □, `y` top = △, `back`, `start`, `leftshoulder`, `rightshoulder`, `lefttrigger`,
`righttrigger`, `dpup` …, `leftx`, `lefty` …) and the device's `bN` button, `aN` axis (`+aN` / `-aN` half, `~`
inverted) or `hN.M` hat direction; `platform:` limits the line to one system (`Mac OS X`, `Windows`, `Linux`). Make
one with the "SDL2 Gamepad Tool" (General Arcade) or SDL's `testcontroller`, or copy a line from
SDL_GameControllerDB; the GUID of your pad is in the log (below).

Where to put it:

- **`gamecontrollerdb.txt` in the config folder**, beside `save.json`: `~/.config/openkumitebeast/` (Linux, macOS),
  `%APPDATA%\openkumitebeast\` (Windows), `$XDG_CONFIG_HOME/openkumitebeast/` when set; `OKB_SAVE_DIR=<dir>` makes it
  `<dir>/openkumitebeast/`. Read at start; blank lines and `#` comments are skipped; SDL's `crc:` field is left out (gilrs
  does not know it). A line gilrs would not use is logged with its line number and why (`gamepads: …:3: "jump" is not
  one of SDL's button / axis names; line not used`); lines for another `platform:` are counted and left out. The
  file's lines count for the pads the bundled database does not know: gilrs puts its database over them (it is logged
  when that happens).
- **`SDL_GAMECONTROLLERCONFIG`**: the environment variable with one or more lines (newline-separated), read by gilrs
  at start, over the bundled database and the file. Use it to replace a bundled mapping:
  `SDL_GAMECONTROLLERCONFIG="$(cat mypad.txt)" ./okbeast`.

The browser has no mapping file (the page has no files); a pad without the standard mapping uses the default layout.

## Testing with a real pad: the log

Run from a terminal (`okbeast …`; the browser: the developer console). Each pad logs when it connects, when it gets a
player and when it goes away:

```
gamepads: 1 mapping(s) for Mac OS X from /Users/me/.config/openkumitebeast/gamecontrollerdb.txt (0 for other systems left out)
gamepad 0 "DualSense Wireless Controller" connected (GUID 050000004c050000e60c000000010000): mapping "PS5 Controller" from the SDL database
gamepad 1 "USB Gamepad" connected (GUID 03000000790000000600000000000000): no mapping, the default layout (buttons in HID order)
gamepad 0 "DualSense Wireless Controller" is player 1
gamepad 1 "USB Gamepad" is player 2
gamepad 0 "DualSense Wireless Controller" went away (player 1 kept for it)
gamepad 2 "DualSense Wireless Controller" connected (GUID 050000004c050000e60c000000010000): mapping "PS5 Controller" from the SDL database
gamepad 2 "DualSense Wireless Controller" is player 1
gamepad 3 "SpaceMouse Compact" not read: 2 buttons, no gamepad (an SDL mapping for it makes it one)
```

The mapping's source is one of: the SDL database, the user's gamecontrollerdb.txt, SDL_GAMECONTROLLERCONFIG, or "no
mapping" with the layout used. In the browser the line says "the standard layout" or "no standard mapping, the
default layout". To check the buttons, the input display (debug tools) or the front end's menus show what arrives.
