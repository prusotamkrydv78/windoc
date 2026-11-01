# Panel roadmap: every daily tool, organised

Goal: the open notch panel becomes the place for everyday computer tasks. It covers every
Vayli tab (see `docs/design-reference/vayli-features/`), organised better, and is built one
tab at a time.

## Organisation

The dock groups tabs by what they are for, separated by thin dividers. Each workspace (Home /
Work / Code) chooses which tabs it shows.

| Group | Tabs |
|---|---|
| **Now** | Home (overview cards) |
| **System** | System (battery, network, performance, volume) |
| **Time** | Clock (local and world clocks), Timers (Pomodoro, countdown, stopwatch, water), Calendar (agenda and reminders) |
| **Info** | Weather (now, hourly, 5-day) |
| **Capture** | Clipboard (history and preview), Notes (rich checklists), Shelf (parked files), Drop (phone transfer) |
| **Tools** | File converter (images to JPG/PNG/PDF/ICO), Units (length to currency), Links (search engines and bookmarks), Message (notch banner) |
| **Insights** | Screen time (today, 7 days, by app), AI coding (Claude Code / Codex usage), Analytics (website visitors; optional, needs a provider) |

## Tab status and order of work

| # | Tab | Vayli features to match | We have | Work to do |
|---|---|---|---|---|
| 1 | **System** | Battery % + health + charging bar; network name, Wi-Fi standard, link speed, live ↓/↑ with sparklines; CPU/RAM/disk % with sparklines; volume slider + output device | Nothing as a tab (CPU/RAM/disk only as Home rings) | New tab |
| 2 | **Clock** | Large local time, full date, timezone; 12/24 h, seconds, date style; world clocks with offsets and "tomorrow"; add city | Basic world clocks | Rebuild layout and add settings |
| 3 | **Timers** | Mode list (Pomodoro / Countdown / Stopwatch / Water) with live values; big ring; cycle dots; "Break next"; reset / play / skip; steppers; auto-start; completed today | Focus page (Pomodoro) | Add modes, steppers and stats |
| 4 | **Calendar** | Week strip with event dots; "Up next" with time-until; agenda by day; reminders with checkboxes; add reminder | Agenda list from feeds | Week strip, Up next, reminders |
| 5 | **Weather** | Now card (sunrise/sunset, updated), hourly line chart + humidity/wind/UV, 5-day range bars | Detail rows and hourly | Restyle to three cards |
| 6 | **Clipboard** | Filter tabs with counts; pinned / today groups; image thumbnails; preview with pin, delete, copy and word count | List and preview | Tabs, groups, thumbnails |
| 7 | **Notes** | Pinned / this week list; toolbar (B I U S H1 H2, list, checklist, colour, pin, copy, delete); word count | Editor | Toolbar and groups |
| 8 | **Shelf** | Multiple shelves; big file icons with size; sort, select, copy all, clear; drop zone | Shelves | Grid view and actions |
| 9 | **File converter** | Queue of files; JPG / PNG / PDF / ICO; quality or target size; resize; save to; estimated savings; Convert | Nothing | New tab (uses the `image` crate) |
| 10 | **Units** | Categories (length, mass, temperature, data, volume, speed, time, currency); from/to with swap; copy; related rates; recent | Palette converter | New tab on the same engine |
| 11 | **Links** | Search box with engine chips (Google, YouTube, GitHub, Stack Overflow, ChatGPT, Perplexity); saved links grid; add link | Links list | Search and chips |
| 12 | **Message** | Live preview; text up to 60 characters; Flash / Clear; animation (slide, fade, type-on, scroll, blur, pulse) and speed; colour + finish (aurora); duration; repeat | Static status message | New tab and animations |
| 13 | **Screen time** | Today total; time left of the limit (editable); apps used; last 7 days bars with average; by-app list with icons, bars and % | Report rows and week | Restyle to three cards |
| 14 | **AI coding** | Tokens, API value, messages, tool calls, sessions, project, streak; month cost + budget; active-hours bars; by model | Usage report | Cards and charts |
| 15 | **Home** | Overview cards from the tabs above | Cards | Refresh last, once the tabs exist |
| — | Analytics / Revenue | Need an account with a provider (Plausible / Stripe) | — | Later, optional (skipped earlier) |

## Rules for every tab

- **Header:** tab name, a divider, then a context subtitle (date, count, city, "Power · Network · Audio").
  Tab-specific controls go in the header centre.
- **Cards:** `#141414` on black, radius 20, no border. Large numbers (about 34–44 px) with one grey line under them.
- **Primary action:** a white pill. Secondary actions: dark grey circles. Steppers: "− Focus 25m +".
- **Data:** real data only; an empty state explains how to turn the feature on.
- **Performance:** slow system queries run on the `system_watch` worker, never on the UI thread.
