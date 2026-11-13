# The NETWORK menu

Two players connect and fight from the game's own menus, without command-line or URL options. An **engine addition**:
the console game has no network play; the menu is drawn in the style of the game's list menus (`menus::frame`, the
scrolling banner, the 15-px menu font and its palettes, the `>` cursor) with their sounds (0x0A move, 0x0B value,
0x0C confirm, 0x0D back). The netplay underneath is the existing one ([`netplay.md`](netplay.md)): delay-based
lockstep, protocol `BR2N`, UDP or WebRTC rooms. Code: `src/app/net_menu.rs` (the page), `src/app/netplay.rs` (the
app's side).

## The mode menu's row

NETWORK is a row of the title's mode menu: ARCADE, VS, STORY, SURVIVAL, WATCH, CUSTOM, REPLAY, **NETWORK**, OPTIONS,
EXIT GAME (the browser: no EXIT GAME). The game's rows keep their order; the engine's REPLAY and NETWORK sit
between CUSTOM and OPTIONS and EXIT GAME stays last, so the wrap from ARCADE upwards reaches the same rows as before
(EXIT GAME, then OPTIONS). The mode menu shows only the row under the cursor, so no screen of a route that never
visits the row changes; `--locked` (the game's own progression) keeps the row, as it keeps REPLAY and EXIT GAME.

The rows are simulated state (the cursor counts them), so the row is part of the session's start:
`SessionStart::network` (a replay header field, default false), copied to `GameState::network` and
`Front::network` (`frontend::screen::mode_rows`). The app sets it for the sessions it starts when its build can
play over a network (all but a browser build without `netplay-webrtc`), and so does `fetest` (it takes the app's
start, `open_kumite_beast::session_start`: a FESCRIPT going DOWN 7 rows prints `network`); every older replay, `sim`'s
direct fights and the command line's netplay sessions have it off and run the rows they had. The row is a
simulation change: `SIM_VERSION` 8 adds it, and only a session of v8 or later with the flag has it (a session at
`OKB_SIM_VERSION` 7 or lower, or a v7 replay, keeps the old rows), so an older build refuses a new replay as
newer instead of playing it with the old rows; `STATE_VERSION` is 6 (`GameState::network`,
`frontend::Event::Network`).

## Screens and flow

```
mode menu ── NETWORK ──> NETWORK: HOST / JOIN / SETTINGS / EXIT          (△ or EXIT: back to the mode menu)
   HOST ──> the connect's screen: ROOM CODE K7QD (or YOUR ADDRESS 192.168.1.5:7000), where the connect is; △ cancels
   JOIN ──> ROOM CODE (or HOST ADDRESS) on the character picker ── OK ──> the connect's screen; △ cancels
   SETTINGS ──> CONNECTION, SIGNALING SERVER, STUN / TURN SERVER, TURN USER / PASSWORD, HOST PORT, INPUT DELAY, EXIT
connected ──> both players on VS's CHARACTER SELECT (P1 the host, P2 the joiner), the VS screen, the fight, the
              select again after it (as VS does); △ on the select: the mode menu, where NETWORK leaves the session
left / stopped ──> back on the NETWORK menu (with the message when it stopped), the session from before as it was
```

- **The character picker.** A grid of the field's characters plus UPPER / LOWER (the letters' case, URL fields
  only), DEL and OK; the d-pad moves (wrapping), □ × ○ enter the cell, △ deletes the last character (on an empty text
  it cancels), START is OK. It needs only pad bits, so the keyboard, gamepads and Android's touch d-pad and buttons
  all drive it; no system keyboard.
- **Room codes.** The host's are 4 characters of `ACEFHJKMNPRTUVWXY3479` (no 0 / O / D, 1 / I / L, 2 / Z, 5 / S,
  8 / B, 6 / G pairs), random per HOST. The joiner types up to 8.
- **The host's address** (JOIN by direct IP): `A.B.C.D:PORT`, or `A.B.C.D` for port 7000; parsed as an IP address
  only (`net_menu::host_address`), never looked up as a name, so no resolver is asked and the frame loop never waits.
- **Connect.** HOST and JOIN run the command line's connect (`net::Connect`) from the frame loop; its line (waiting
  for the other player, connecting, joining) is on the screen with its seconds. The offered session is the app's own
  start as a new session would have it (the stored memory card, a new seed) with `Entry::Versus` (VS's character
  select at once) and the NETWORK row; the joiner adopts the host's, as on the command line. Once through, the live
  session is put aside (as for a replay played from the list) and the agreed one begins.
- **Leaving.** NETWORK on the mode menu during the session: both sides step the same press (`Request::Network`) and
  end it (this side's last inputs sent before the Bye); OKB_FAST / OKB_FEFAST, off during the session, are back
  after it; the one whose Bye arrives first may show "the other player left (player n went back to the NETWORK
  menu)". EXIT GAME quits both apps, as on the command line.
- **Errors**, each a message on the NETWORK menu (OK: back): no signaling server set (HOST / JOIN by room code
  before SETTINGS has one), a setting that does not check out (an URL, the port; the picker stays open with why), the
  debug tools on (their cheats would change one side only), the link or server unreachable, no answer from the host
  (15 s), nobody in the room (300 s), the host refusing another build (`PROTO`, `SIM_VERSION`, `STATE_VERSION`, the
  build), and in the session: the other player left, nothing from them for 10 s, a desync (after its dump is
  written).

## Settings

`NetPrefs`, saved as `net.json` beside the save (Android: in the app's storage; the browser: localStorage
`okb_net`) whenever one changes. These are the app's own preferences: not `frontend::options::Prefs` (which is part
of the logic's state and of the handshake), never in a replay, a save or a netplay packet; netplay itself still
never writes the card or the logic's preferences. The file is plain JSON: a TURN PASSWORD set there is stored as
typed, readable by anyone who can read the save folder (or the browser profile's localStorage).

| Setting | Default | |
|---|---|---|
| CONNECTION | ROOM CODE (DIRECT IP in a build without `netplay-webrtc`) | ROOM CODE: WebRTC through a matchbox signaling server (desktop with the feature, the browser, Android); DIRECT IP: UDP (desktop, Android; not the browser) |
| SIGNALING SERVER | empty | `ws://HOST:PORT` of the player's own `matchbox_server`; HOST / JOIN by room code ask for it first |
| STUN SERVER, TURN SERVER, TURN USER, TURN PASSWORD | empty | none: no third party is contacted; without one only this machine and the LAN connect |
| HOST PORT | 7000 | the UDP port a direct-IP host listens on (every interface; it shows its address on the network) |
| INPUT DELAY | AUTO | AUTO: the host's choice from the round trip (`--delay`'s default); 1..30 frames |

## Where the state lives

| What | Where | Simulated? |
|---|---|---|
| the NETWORK row, the press that opens the menu or leaves the session | `Front` (mode menu), `Event::Network` → `Request::Network` | yes (the start's `network` flag) |
| the NETWORK menu, the picker, the connect's screen | `App::net_ui.menu` (`app/net_menu.rs`) | no: an app page, the session waits on the mode menu |
| the settings | `net.json` / localStorage | no |
| the netplay session | `App::net` (`net::Netplay`), the put-aside session in `App::live` | the session's own `GameState` (a new one per connect) |

## Debug hooks

`OKB_NETCODE` (the host's code), `OKB_NETBIND` (a direct-IP host's address, e.g. `127.0.0.1`), `OKB_NETSHOTS`
(session frames saved), `OKB_NETEXIT` (a shot of the menu after the session, then quit); `OKB_UISCRIPT` /
`OKB_UISHOTS` drive and capture the menu, `OKB_NETSCRIPT` the session ([`debug_hooks.md`](debug_hooks.md)).
