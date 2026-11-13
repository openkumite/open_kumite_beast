# Netplay

Two players, delay-based lockstep (`src/net/`); the session (memory card, unlocks, options, menus) is the host's, and
netplay never writes the save. Both builds must be the same (a mismatch is refused with a message). An engine addition.

## Playing

- In the game: the mode menu's NETWORK (HOST / JOIN by room code or direct IP, SETTINGS), on the desktop, in the
  browser and on Android: [`network_menu.md`](network_menu.md). The options below keep working.
- UDP (desktop): `okbeast --host 7000` and `okbeast --join HOST-ADDRESS:7000` [`--delay N`].
- Rooms over WebRTC (the browser build, and desktop builds with `--features netplay-webrtc`): both players open the
  same room on a [matchbox](https://github.com/johanhelsing/matchbox) signaling server; the first one in hosts.
  No server is built in: run your own `matchbox_server` (`cargo install matchbox_server --version 0.15.0`, the
  protocol of the pinned `matchbox_socket`). Desktop: `okbeast --signal ws://HOST:3536 --room CODE`; browser:
  `…/play/?signal=ws://HOST:3536&room=CODE`. No STUN / TURN server is used unless given (`--stun stun:HOST:PORT`,
  `--turn turn:HOST:PORT --turn-user U --turn-pass P`; `?stun=`, `?turn=&turnuser=&turnpass=`): without one only the
  same machine and the LAN connect.
- The team battle (four fighters) is not offered in a netplay
  session: its mode menu has no TEAM BATTLE row (`mode_rows` in `frontend/screen.rs`, by `SessionStart::netplay`).
  Netplay with up to four peers is its phase 2, not started.
- `cargo run --release --bin nettest -- ../okb_export` runs a whole session between two headless peers (UDP; with
  `NETTEST_SIGNAL=ws://127.0.0.1:3536` a room; `OKB_NETSIM=lat_ms,loss_pct` simulates the network).

## How it works

- **Lockstep, no prediction.** Port 1 is the host, port 2 the joiner; each side's pads drive only its port. Frame f
  is stepped only once both inputs for it are there; the input for frame f + delay is taken when frame f is stepped,
  and every packet carries all the input the peer has not acknowledged, so a lost packet is covered by the next.
  The delay is `--delay N` or chosen by the host from the measured round trip.
- **The same session on both sides.** The joiner adopts the host's `SessionStart` (seed, memory card, unlocks,
  entry, browser menus), its logic preferences and a direct fight's costumes; the host's card carries the joiner's
  own CONTROLLER CONFIG (its P1 map) as PLAYER 2's, and each extra stage (Q3DM17, Q3DM6) is on its stage select only
  when both sides have the stage's files (the joiner's Hello says which it has, one bit each;
  [`extra_stage_q3dm17.md`](extra_stage_q3dm17.md), [`extra_stage_q3dm6.md`](extra_stage_q3dm6.md)). Loads and fetches are inputs too: each side reports "done" in its
  input stream, and `FrameInput::LOADED` / `FETCHED` is applied at the first frame both have reported. Both sides record
  the session as a replay, the same bytes on both.
- **Desync detection.** Every `session::replay::CHECK_EVERY` (60) frames each side sends its state hash; a hash
  that differs stops both with "desync at frame F", the section hashes (`session::section_hashes`) exchanged and
  logged and the state dumped (`Netplay::write_desync`, in the save folder). This is why the simulation must be
  deterministic across targets: [`determinism.md`](determinism.md).
- **Off in netplay:** the debug tools, the debug key cheats and the env cheats (a `Cheat` would change one side
  only), the card and preference writes, the replay list. The in-fight pause is the game's own (START is an input).

## Code

| File | What |
|---|---|
| `src/net/mod.rs` | `Netplay`: the lockstep (input queues, acks, the load reports, the hash checks, frame advantage, timeouts) |
| `src/net/proto.rs` | the packets (`MAGIC` "BR2N", `PROTO`, `Msg`, `encode` / `decode` with caps on every length) |
| `src/net/handshake.rs` | Hello / Welcome / Sync / Chunk / Ready / Go: versions checked, the start blob sent in chunks |
| `src/net/connect.rs` | brings a link up and runs the handshake, polled from a frame loop |
| `src/net/link.rs` | the `Transport` trait, an in-memory pair (tests), `NetSim` (`OKB_NETSIM`) |
| `src/net/udp.rs`, `src/net/webrtc.rs` | the transports (UDP desktop; a matchbox room, feature `netplay-webrtc`) |
| `src/app/netplay.rs` | the app's side: the connect screen, the step's input from the lockstep, the panel, the desync dump; the NETWORK menu's connect, session and return |
| `src/app/net_menu.rs` | the NETWORK menu's page: HOST / JOIN / SETTINGS, the character picker, the connect's screen, `NetPrefs` |
| `src/bin/nettest.rs` | two headless peers in threads; exit 0 when the match finished in sync and both replays are byte-equal |

The packet layout and its version `PROTO` (5) are documented in `src/net/proto.rs`'s header; the handshake refuses a
peer whose `PROTO`, `SIM_VERSION`, `STATE_VERSION` or build (`platform::support::VERSION`) differs, and one whose game
data's content id (`SessionStart::content`, the Hello's `content`) differs from the host's: "the other player's game
data differs: convert it again" (`handshake::DATA_DIFFERS`). A new message:
[`../CONTRIBUTING.md`](../CONTRIBUTING.md#6-a-new-netplay-message).
