# Reporting a problem

1. When it happens, press **F8**. This writes a support report (`okb_report_<time>.txt` in the folder the game was started in, or in the save's folder when that one cannot be written, e.g. the macOS app; a message on screen shows where it went; on the web it goes to the browser console, F12 → Console). The report holds the build, the platform, what was on screen (menu, round state, both fighters) and the last 400 log lines.
2. Press **F7** for a screenshot (`okb_shot_<time>.png`, desktop only).
3. If the game crashed, send the `okb_crash_<time>.txt` file it wrote (in the same place as the support report).

Send the report, the screenshot and a sentence about what you did just before.

## Debug keys

| key | does |
|---|---|
| F1 | overlay: build, fps, screen, round state, both fighters (move, anim, hp, gauge, position, pad) |
| F2 / F3 | leave P2 / P1 at 1 HP (the next hit is a KO) |
| F4 | round timer to 1 second |
| F6 | full beast gauges |
| F7 | screenshot |
| F8 | support report |
| F9 / P | pause, `.` steps one frame while paused |
| F10 | fast forward ×4 |
| F11 | unlock everything for the session (characters, costumes, movies, pictures) |

## Options

Desktop: `okbeast --help`. `--debug` starts with the overlay, `--locked` keeps the game's own progression (characters and
fourth costumes as the save unlocked them, the game's nine stages; by default all characters and costumes are
selectable and the stage select adds EXTRA1..EXTRA4), `--fight --p1 YUG --p2 ALI --stage ST0`
skips the menus. More detail in the log: `OKB_LOG=debug`.
Web: add `?debug=1`, `?locked=1`, `?log=debug`, or `?fight=1&p1=YUG&p2=ALI&stage=ST0` to the page address.
