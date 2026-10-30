# UI and UX Design Guide for `winisland-next`

This guide defines an original interface system for the native Rust-only `winisland-next` application. It should be cleaner, more adaptable and easier to understand than existing notch applications without imitating Vayli’s exact appearance.

The central idea is a calm top-edge workspace: the resting surface communicates only what matters now, while the expanded workspace reveals tools in a predictable structure. Beauty comes from proportion, typography, motion and restraint rather than decoration.

## Implementation constraint

All desktop UI behavior, layout, animation, state and rendering must be implemented in Rust through the existing Skia, D3D12 and DirectComposition stack. Do not use Python/PySide, React, TypeScript, HTML/CSS, WebView or Tauri for the product interface. Images, fonts, language catalogues and shaders may remain external resources.

Separate domain state from pixels:

```text
Module state → view model → layout tree → animation state → Skia draw commands
```

Module services must never draw directly. The renderer must never fetch data, access storage or invoke Windows services during a frame.

# Experience goals

## The interface must feel

- Immediate without being restless
- Dense enough for power users without appearing crowded
- Native to Windows without looking like a Settings clone
- Personal without becoming playful by default
- Private and trustworthy when displaying sensitive information
- Consistent across modules even when their data differs

## The interface must not

- Copy the shape, colors, layout or copywriting of Vayli
- Reproduce Apple’s Dynamic Island pixel for pixel
- Show continuous animation merely to look alive
- Hide essential actions behind unexplained gestures
- Put every available metric on the Home screen
- Use color as the only indicator of state
- Show financial, clipboard, note or calendar content during screen sharing without an explicit preference

# Original design direction

Use a **calm ribbon** rather than a literal hardware notch. The resting form is a narrow floating capsule attached visually to the top edge by light and spacing. When opened, it unfolds downward into a layered workspace whose content appears to slide from the ribbon rather than from a black cutout.

Distinctive characteristics:

- A soft inner highlight that identifies the active Context Set
- A narrow “signal rail” for timers, media, agent status and alerts
- Cards that use restrained depth instead of heavy borders
- A compact command field that can operate every module
- Privacy-aware presentation mode built into the shell
- Optional companion represented as a small ambient glyph, not a large cartoon by default

# Information architecture

## Primary levels

```text
Resting ribbon
├── Current activity
├── Status indicators
└── Reveal affordance

Expanded workspace
├── Context header
├── Command field
├── Navigation rail
├── Active module surface
└── Quick actions

Settings
├── Modules
├── Appearance
├── Behavior
├── Privacy and data
├── Integrations
├── Plugins
├── Backup and restore
└── About and diagnostics
```

The expanded workspace should always answer three questions:

1. Where am I?
2. What can I do here?
3. How do I return or close it?

## Default navigation

Use a horizontal bottom dock for the primary dashboard, matching the top-edge panel’s wide geometry. A compact vertical rail remains an option for a detached or full settings window.

Default destinations:

1. Home
2. Media
3. Clipboard
4. Shelf
5. Focus
6. More

These match the first-run modules in the engineering workflow. Settings is reached from the header gear, not the dock. “More” opens the complete searchable module grid. Users can pin any module into the primary navigation, up to eight destinations at standard width. In the compact size class the dock shows Home, Focus and More, and the rest move into More.

# Surface states

## Hidden

No visible surface. Global shortcut and tray icon remain available. Use this state for user choice, exclusive fullscreen and presentation suppression.

## Resting

Recommended size range:

- Width: 132–220 logical pixels
- Height: 28–34 logical pixels
- Top inset: 4–8 logical pixels when floating
- Minimum pointer target: 32 logical pixels high through an invisible safe hit region

Resting content is limited to one principal item and up to three tiny state indicators.

## Live activity

The ribbon may widen temporarily for:

- Track change
- Timer completion
- Clipboard confirmation
- Volume/brightness change
- Agent waiting for input
- Transfer progress

The activity collapses automatically but remains available in a recent-activity list.

## Peek

Peek is a lightweight card for a single action, such as pausing a timer or copying the last clipboard item. It must not expose the entire application.

## Expanded

Recommended ranges:

- Compact width: 420–520 logical pixels
- Standard width: 600–760 logical pixels
- Maximum height: 75% of working-area height
- Minimum edge clearance: 12 logical pixels

Expanded size adapts to module content but should not jump when navigating between ordinary modules. Use two stable size classes rather than a different window size for every page.

Size-class selection uses the work area of the current monitor in logical pixels:

| Work area | Size class |
|---|---|
| Width ≥ 800 and height ≥ 480 | Standard |
| Otherwise | Compact; content scrolls vertically within the 75% height limit |

Reference points:

- 1280 × 720 at 150% scaling is 853 × 480 logical, so Standard.
- 1280 × 720 at 200% is 640 × 360 logical, so Compact. That configuration is below the supported minimum but must remain usable.

## Pinned workspace

Pinned mode behaves like a small tool window. It may move away from the top edge, stays visible by user choice and uses a visible title/drag region. Never confuse pinned mode with the transient hover surface.

# Layout model

## Standard expanded dashboard

```text
┌──────────────────────────────────────────────────────────┐
│ Product  Home   Date              Battery  Privacy  Gear │
├──────────────────────────────────────────────────────────┤
│ ┌────────────┐ ┌────────────┐ ┌───────────────────────┐ │
│ │ Screen time│ │ Weather    │ │ Focus                 │ │
│ └────────────┘ └────────────┘ └───────────────────────┘ │
│ ┌────────────┐ ┌────────────┐                           │
│ │ Clipboard  │ │ Notes      │     Contextual space      │
│ └────────────┘ └────────────┘                           │
├──────────────────────────────────────────────────────────┤
│ Home    Media    Clipboard    Shelf    Focus    More     │
└──────────────────────────────────────────────────────────┘
```

This layout is a functional interpretation of the supplied dashboard reference, not a pixel-for-pixel copy. Preserve the useful hierarchy while creating original proportions, iconography, motion, colors and card composition. On a narrow surface, cards become a single scrollable column and less-used dock destinations move into More.

## Density modes

- Comfortable: default; larger spacing and labels
- Compact: information-dense for experienced users
- Touch: larger targets and simplified hover behavior

Density changes spacing and target size, not information hierarchy.

# Design tokens

Implement tokens in the renderer rather than hardcoding module colors.

## Spacing

Use a four-pixel base grid:

| Token | Value | Use |
|---|---:|---|
| `space-1` | 4 | Icon gaps and internal micro-spacing |
| `space-2` | 8 | Tight control padding |
| `space-3` | 12 | Standard control padding |
| `space-4` | 16 | Card padding |
| `space-5` | 20 | Page section spacing |
| `space-6` | 24 | Major groups |
| `space-8` | 32 | Large separation |

## Radius

| Token | Value |
|---|---:|
| Small controls | 7 |
| Inputs and rows | 10 |
| Cards | 14 |
| Expanded workspace | 20 |
| Resting ribbon | Fully rounded |

Avoid giving every nested element a rounded container. Use padding and typography before adding another card.

## Typography

Use the Windows UI font stack, beginning with Segoe UI Variable where available.

| Style | Size | Weight | Use |
|---|---:|---:|---|
| Display | 28 | 600 | Rare empty state or onboarding title |
| Page title | 20 | 600 | Module heading |
| Section title | 15 | 600 | Content groups |
| Body | 14 | 400 | Primary text |
| Label | 12 | 500 | Controls and metadata |
| Caption | 11 | 400 | Secondary timestamps and sources |
| Numeric | 20–32 | 550 | Timers and key measurements |

Rules:

- Use sentence case.
- Keep line length under approximately 70 characters in notes and explanations.
- Use tabular numerals for timers, clocks and changing measurements.
- Never reduce important text below 11 logical pixels.

## Color system

Define semantic tokens for dark, light, high-contrast and translucent modes:

```text
surface/base
surface/raised
surface/overlay
surface/interactive
text/primary
text/secondary
text/disabled
border/subtle
border/strong
accent/primary
accent/soft
state/success
state/warning
state/error
state/info
privacy/sensitive
```

The accent color comes from the active Context Set, not every module. Modules may use small semantic chart colors but must not recolor the entire workspace.

Maintain at least WCAG AA contrast for ordinary text. Translucency must fall back to an opaque surface when the background makes content unreadable.

## Elevation

Use three levels:

- Base ribbon: subtle ambient shadow
- Expanded workspace: directional shadow plus thin inner highlight
- Floating menu/dialog: stronger shadow and opaque surface

Avoid multiple stacked shadows or glowing every edge.

# Motion system

Motion communicates cause and hierarchy.

## Timing

| Motion | Duration |
|---|---:|
| Hover/press feedback | 80–120 ms |
| Small state transition | 140–180 ms |
| Ribbon expansion | 180–240 ms |
| Page navigation | 160–220 ms |
| Dialog/sheet | 180–240 ms |
| Success confirmation | 300–500 ms total |

Use spring motion only for the shell’s geometry and direct manipulation. Lists, dialogs and charts should use restrained easing.

## Reduced motion

When Windows reduced motion is active:

- Replace spring expansion with short crossfade and size transition
- Disable spectrum, parallax, cursor-following light and companion gestures
- Keep progress and state changes visible without animation
- Never require animation to understand where content moved

# Component library

## Ribbon indicator

One icon, one short line and optional progress. Long text truncates with a tooltip in expanded view, not an endlessly scrolling marquee.

## Command field

Supports search, commands and natural shorthand. It displays scope and destination before executing actions with external effects.

States:

- Idle suggestion
- Local results
- Online-provider result
- Destructive confirmation
- No result with recovery suggestion

## Module card

Contains a heading, optional status, content and no more than two visible actions. Additional actions go into an overflow menu.

## Metric tile

Contains one primary value, label, small trend and timestamp. Do not place multiple unrelated metrics in one tile.

## List row

Minimum height:

- Comfortable: 44 logical pixels
- Compact: 36 logical pixels
- Touch: 52 logical pixels

Selection, hover, keyboard focus and drag state must look different.

## Empty state

Explain why the area is empty and offer one primary action. Do not use generic illustrations where a concrete example would teach faster.

## Permission sheet

Explain:

1. What the feature observes
2. Why it needs the information
3. Whether data leaves the device
4. How to disable and delete it

Buttons use “Enable screen time” or “Choose location,” not a vague “Allow.”

## Sensitive-content treatment

Clipboard, notes, calendar, revenue and analytics can show a privacy veil. The veil preserves layout while replacing values with neutral shapes. A single temporary reveal control requires deliberate activation.

# Navigation and input

## Pointer

- Hover may reveal Peek but must not trigger destructive or network actions.
- Click opens or activates.
- Drag operates only on visible drag handles or draggable content.
- Scrolling over the resting ribbon must not unexpectedly change system volume unless explicitly enabled.

## Keyboard

Recommended defaults:

| Shortcut | Action |
|---|---|
| `Ctrl+Alt+Space` | Open command field |
| `Ctrl+Alt+W` | Open workspace (avoid `Ctrl+Alt+Arrow`, which some Intel graphics drivers reserve for screen rotation) |
| `Esc` | Close current layer, then workspace |
| `Ctrl+Tab` | Next pinned module |
| `Ctrl+Shift+Tab` | Previous pinned module |
| `/` | Focus search within a module |
| `Ctrl+,` | Settings |

Shortcuts must be editable and conflict-checked.

## Focus rules

- Passive activities never take focus.
- Opening by keyboard focuses the command field or first meaningful control.
- Opening by pointer preserves the current application focus until an editable control is clicked.
- Closing restores focus to the previously active application where Windows permits.

The resting surface is a non-activating topmost window, so these rules depend on the focus model defined in Phase 4 of the engineering workflow. Do not implement keyboard entry points before that model exists.

## Touch

Use click/tap parity. Do not make swipe the only way to reach a page. Provide visible navigation and use touch-friendly targets.

# Onboarding

Use four short steps:

1. Explain the ribbon and demonstrate open/close.
2. Choose three starter modules.
3. Configure one shortcut and monitor.
4. Review privacy choices with all sensitive access disabled initially.

Then open a sample Home populated with clearly labeled demo content. Remove demo content as soon as real data exists.

Do not ask for location, notification access, startup registration, clipboard monitoring and screen-time tracking in one permission wall.

# Module-specific interface specifications

## Home

Use a responsive card grid with one emphasized “Now” card and smaller summaries.

- First row: current activity, next event or active focus timer
- Second row: user-selected summaries
- Final row: recent items and quick actions

Editing mode uses visible drag handles and an “Add card” command. Normal mode has no persistent layout controls.

## Media

Compact state shows artwork or source glyph, title and progress. Expanded state uses artwork as a subtle color source, not a full blurred background that harms readability.

Place playback controls in a stable row. Seeking and output-device selection are secondary. Lyrics, when enabled, occupy their own page.

## Clipboard

Use a two-pane layout at standard width:

- Left: searchable chronological list
- Right: preview and actions

At compact width, preview opens as a detail page. Display type, source application when available, capture time and pin state. Sensitive items can be individually protected.

## File Shelf

Show shelves as named tabs or a compact selector. Files use a list/grid toggle, clear managed-copy/reference indicators and visible expiration. Drop target appears only during drag or after choosing Add.

## Notes

Use a note list and distraction-free editor. Checklists retain large hit targets. Revision history is a side sheet, not permanently visible.

Quick capture from the command field should save without forcing the full Notes module open.

## Links and search

The command field is the primary interface. Saved links appear as concise rows with domain, icon and optional tag. Search-provider selection is visible before an online query is sent.

## Focus and timers

The main timer is visually dominant, with task label, remaining time and three controls: pause/resume, finish and more. Session history and templates sit below, not beside the countdown.

Use calm progress geometry. Avoid making the entire interface flash at completion.

## Calendar

Default to agenda plus a compact date strip. Month view is available but not the first view inside a small surface. Event privacy controls allow title, generic “Busy,” or hidden compact display.

## Clock

World clocks appear as aligned rows with day offset. The planner uses a shared horizontal time ruler and highlights unsuitable local hours without blocking selection.

## Weather

Lead with current condition, perceived temperature and the next meaningful change. Hourly detail uses a readable line chart; do not show every available meteorological measurement simultaneously.

## Units

Use two large value fields with unit selectors and reversible direction. Recent conversions appear below. Currency shows rate time and source.

## File converter

Use a three-step flow on one page:

1. Input files
2. Output recipe
3. Destination and convert

Show estimated output, conflict policy and metadata-removal state before conversion. Progress is itemized, cancellable and recoverable.

## Screen time

Show total active time, category distribution and application list. Window-title detail is a separate opt-in view. Avoid red/green moral judgments and competitive productivity scores.

## System monitor

Use a consistent metric tile pattern and allow only selected charts to animate. The default page shows four key resources; detailed process information opens separately.

## Message and presence

Compose text, choose a calm style, preview exactly how it will appear and set an expiry. Show whether the message will be visible during screen sharing.

## Local transfer

Separate Send and Receive clearly.

Send flow:

1. Choose files
2. Show paired-device QR and optional PIN
3. Confirm connected device
4. Show per-file progress and session expiry

Receive flow:

1. Choose destination
2. Pair device
3. Review incoming names and sizes
4. Accept or reject

Never start listening on the network merely by opening the module’s landing page.

## AI coding usage

Show sessions, messages/tokens and model distribution while hiding project paths by default. Agent state uses four consistent states: working, waiting, completed and failed.

Cost estimates must include a visible information control explaining assumptions.

## Analytics

Lead with current visitors and selected period. Secondary charts show top pages and sources. Always display provider, site and last update to prevent confusion between connected properties.

## Revenue

Use a privacy veil by default when presentation mode is active. Show gross, fees, refunds and net separately. Do not imitate a banking interface or suggest actions that the read-only module cannot perform.

## Companion

The companion occupies the signal rail or a small corner of Home. It may acknowledge completion, breaks and system states but should never cover controls. Frequency, sound, motion and contextual reactions are independently adjustable.

# Settings design

Settings uses a full window when necessary rather than forcing complex configuration into the transient ribbon.

## Modules

Each module row displays:

- Enabled state
- Short purpose
- Data category
- Network status
- Navigation pin state
- Configure action

Support drag ordering only after entering an explicit edit mode.

## Privacy and data

Present a module inventory:

| Module | Local data | Network | Retention | Actions |
|---|---|---|---|---|
| Clipboard | Text and images | Never | 7 days | View, export, clear |
| Weather | City and forecast cache | Provider | 24 hours | Change, clear |
| Revenue | Aggregated cache | Stripe/Gumroad | Configurable | Disconnect, clear |

The actual application populates this from module metadata so documentation and behavior cannot silently diverge.

## Integrations

Use connection cards with provider, account/property, granted scope, last successful refresh, test, disconnect and delete-cache actions. Never redisplay a complete API secret.

# Accessibility

## Required behavior

- Every control has an accessible name, role, state and concise description.
- Focus order follows visual order.
- Keyboard focus is always visible.
- Text supports Windows scaling without clipping.
- Charts have text summaries and data tables.
- State does not depend on color alone.
- Live activities use polite announcements and do not repeatedly interrupt screen readers.
- Drag-and-drop actions have keyboard alternatives.
- High-contrast mode uses system colors and removes decorative translucency.
- Touch targets are at least 44 × 44 logical pixels in Touch density.

Test with Narrator, keyboard only, 200% text scaling, reduced motion and Windows high contrast.

## Implementation note

Everything is drawn with Skia into a D3D12/DirectComposition surface, so Windows sees no controls at all. Every requirement above depends on a UI Automation provider tree built from the same semantic layout nodes the renderer draws (see Phase 4 of the engineering workflow). Each shared component must expose its UIA role, name, state and bounds when it is created. Do not plan to add these later.

# Localization

- Keep strings out of render code.
- Support plural rules, locale-aware dates/numbers and right-to-left layout.
- Allow 40% text expansion without clipping ordinary controls.
- Do not concatenate translated fragments into sentences.
- Keep icons culturally neutral and avoid text baked into images.
- Test English, Hindi, a CJK language and Arabic before stable release.

# Error and recovery design

Errors should state what happened, what was preserved and the next action.

Examples:

- “Weather could not refresh. Showing data from 9:30 AM.”
- “The calendar feed is too large to process safely. Your existing events were not changed.”
- “This plugin stopped the app during its last run. It has been disabled.”

Avoid raw stack traces in the interface. Provide a details expander with redacted diagnostics and a Copy button.

# Prototype and implementation workflow

## Stage 1: structure prototype

Create grayscale prototypes for:

- Resting, live activity, peek and expanded states
- Home, Clipboard, Shelf, Focus and Settings
- Permission and privacy sheets
- Narrow, standard and touch density
- A 740 × 300 logical-pixel standard dashboard shell and a 480 × 300 compact shell, both using deterministic sample data

Test hierarchy before color or glass effects.

## Stage 2: interaction prototype

Validate:

- Opening without accidental activation
- Keyboard navigation and focus restoration
- Module switching
- Command field behavior
- Dragging files and clipboard items
- Privacy veil and presentation mode

## Stage 3: visual system

Implement tokens and seven representative components before complete pages:

1. Ribbon
2. Expanded shell
3. Navigation item
4. Card/list row
5. Command field
6. Dashboard card
7. Bottom dock item

Build light, dark, high-contrast and opaque fallbacks at the same time.

## Stage 4: renderer integration

- Define semantic layout nodes independent of Skia drawing code.
- Centralize typography measurement and DPI conversion.
- Use retained rendering and damage tracking.
- Cache icons, shaped text and thumbnails with bounded memory.
- Stop animation clocks when surfaces are hidden.
- Capture frame-time and dropped-frame diagnostics locally.
- Keep all frame callbacks non-blocking; module updates arrive through bounded channels and immutable snapshots.
- Target the monitor refresh rate while animating and return to event-driven redraw when stable.
- Validate 60, 120 and 144 Hz displays with frame-time traces rather than judging smoothness by eye alone.

## Stage 5: functional dashboard slices

Connect one module at a time, following the delivery roadmap in the engineering workflow:

1. Focus timer
2. Notes
3. Clipboard history
4. Screen time
5. System status and battery
6. File Shelf
7. Media (re-skinning the inherited implementation)
8. Weather

Each slice includes loading, empty, disabled, stale, error and permission states before the next module begins.

## Stage 6: usability validation

Test with at least these user profiles:

- Keyboard-heavy developer
- General Windows productivity user
- Multi-monitor creator
- Touchscreen laptop user
- User relying on screen reader or high contrast

Give users real tasks rather than asking whether the interface looks attractive.

# UI acceptance checklist

Before a module is accepted:

- Its primary task is recognizable within five seconds.
- The main action is reachable in two interactions or fewer from the expanded workspace.
- It works at compact and standard widths.
- It works at 200% scaling without clipping.
- Empty, loading, offline, stale, error and disabled states are designed.
- Sensitive content respects privacy veil and presentation mode.
- Keyboard and screen-reader paths are complete.
- Motion has a reduced-motion equivalent.
- The page uses shared components and semantic tokens.
- Background work and refresh state are visible when relevant.
- The interface never implies that a read-only integration can perform writes.

# Final visual quality bar

The product is visually ready when:

- The resting ribbon is useful without demanding attention.
- The expanded workspace feels like one product rather than unrelated widgets.
- Every module has a clear content hierarchy.
- Empty space is used intentionally.
- Glass and blur remain readable against light and complex wallpapers.
- Animations explain state change and stop when unnecessary.
- Privacy-sensitive modules feel trustworthy before the user reads documentation.
- The interface remains distinctive with every decorative effect disabled.

# Reference material

- Active local repository: `C:\Users\Purshotam\Desktop\WinIsland-Product-Blueprint\winisland-next`
- Native UI baseline source: https://github.com/lzsobig/BuddyDesk-pet-claude/tree/main/winisland
- Original WinIsland repository: https://github.com/WinIslandProject/WinIsland
- Vayli functional benchmark: https://vayli.in/features/
- Microsoft Fluent 2 design guidance: https://fluent2.microsoft.design/
- Windows accessibility guidance: https://learn.microsoft.com/windows/apps/design/accessibility/accessibility
- Windows interaction design: https://learn.microsoft.com/windows/apps/design/input/
