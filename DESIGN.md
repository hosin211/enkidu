# Enkidu design system

Enkidu is a developer's own tool for the day's tasks. Every task gets a workspace with a Claude in it; you see at a
glance which Claude works, which one needs you and which one is done, you review the work line by line, and you
bring one workspace's changes into the folder where your dev environment runs (Live). In the oldest story we have,
Enkidu is the companion made to stand beside Gilgamesh: the one who works beside you. The interface should feel
like a well-made Mac tool you keep open all day: calm, dense, exact, and alive where something happens.

**Dark only, in layers.** The window is a frame that holds islands, in a cool blue-black: the frame is the darkest,
each pane (the changes, Claude, a tool panel) is an island one step up with a lit top edge, and menus, dialogs and
toasts float another step up. Color carries meaning: the lamps are Claude's state, signal blue is what is yours, and
small colored icons tell file types and projects apart. There is no light theme. The research behind this (Linear,
Vercel Geist, Radix, JetBrains Islands, Raycast, Emil Kowalski's motion rules, and apps like this one) came first.

All tokens live in `crates/enkidu-ui/src/tokens.rs`. Components use the token names; the only colors computed in code
are the icon hues (`tone()` there). The window's own background is the frame color, so nothing flashes on launch.

## Principles

1. **Layers, not black.** Higher is lighter: frame `#0A0B0E`, islands `#111217`, their title bands `#15161C`,
   floating layers `#191B21`, tooltips `#272A31`. Every layer has a 1 px edge and a lit top edge, drawn over its
   content so a terminal can't cover it. Shadows stay translucent.
2. **Each section stands on its own.** Every pane is an island with a 12 px corner, 8 px from the next; the space
   between two islands is also where you drag to resize them. The island the keyboard is in gets a signal-blue edge
   and a faint glow, so you always know where your typing goes.
3. **Color means something.** The signal lamps are Claude's state (amber working, red needs you, green done). Signal
   blue (`accent`) is what is yours: focus, the tab on screen, remarks, the button that types into Claude. File types, projects and
   terminals carry small colored icons. The diff keeps green and red. Nothing is colored for decoration.
4. **Compact, but said in words.** Signs where they are universal (the lamps, counts, +/−), words where they help:
   "Needs you" on a tab, "Claude is working" and the branch in the status line. With little room, tabs drop their
   extra words first.
5. **Nothing about nothing.** What isn't there takes no room and no words: no "No changes yet", no counts of zero,
   no empty pane, no field for something the keyboard opens (Search), no button twice on one screen. Loading words
   ("Reading the file…", "Starting Claude…") show only when a wait is long enough to notice (0.4 to 0.8 s); a quick
   load goes from nothing straight to the result.
6. **Motion with a job.** Things move to show a state (the working ring, the ping, the running line), to say where
   they came from (menus from their trigger, the tab pill gliding), to answer a press, or to soften a jump (the
   changes rising in, toasts). Strong ease-out curves, under 300 ms. What the keyboard opens many times a day (the
   palette, Search) is instant. `prefers-reduced-motion` stops everything that moves and keeps only fades.
7. **Sentence case, plain verbs.** "Go live", "Type 3 remarks into Claude", never "SUBMIT". No all-caps labels, no
   dot-joined meta strings, no arrows in button text.
8. **Everything remembers.** Window size and position, the layout and its split, the panel and its width, the last
   workspace of every project, the diff layout, unsent remarks and every workspace's changes as last seen all
   survive a restart.
9. **Instant first, fresh second.** Show what we already know right away (remembered changes, branches, diffs), then
   refresh in the background. Never block a click on git.

## Color

| Token | Value | Use |
|---|---|---|
| `frame` | `#0A0B0E` | the window around the islands: the top rows, the gaps, the rail, the status line |
| `canvas` | `#111217` | the islands: terminal, diff, panels |
| `chrome` | `#15161C` | an island's title band, sticky file headers |
| `raised` | `#191B21` | menus, dialogs, the palette, toasts, remark cards |
| `tip` | `#272A31` | tooltips |
| `island-edge` / `island-edge-focus` | white at 7% / signal blue at 60% | an island's edge; the island the keyboard is in (3.47 to 1 against the frame) |
| `highlight` | white at 5% | the lit top edge of every layer |
| `ink` | `#ECEEF2` | primary text |
| `ink-2` | `#A6AAB3` | secondary text, icons in use |
| `ink-3` | `#7C818C` | hints, icons at rest, line numbers (4.6:1 on an island) |
| `rule` / `rule-strong` | white at 6.5% / 11% | hairlines; control edges |
| `hover` / `soft` / `selected` / `selected-hover` / `thumb` | white at 4.5% / 8% / 12% / 16% / 11.5% | the pointer's fill; a ghost button while pressed, a neutral chip; what is chosen (a row, a tab, a chip); the pointer on what is chosen; a segmented control's thumb |
| `accent` / `accent-strong` / `accent-soft` | `#7AA2FF` / `#4F7DFF` / blue at 14% | yours: where you are (the edge of the tab on screen, the bar of the row on screen), where your typing goes, remarks, the button that types into Claude, a new file's flash; the mark's second wedge, the one beside you |
| `behind` | `#646872` | the blue marks of where you are, while the window is behind another app |
| `claude` | `#E8845F` | Claude's coral, on its terminal tab |

Signal lamps:

| Token | Value | Meaning |
|---|---|---|
| `lamp-amber` | `#FFB547` | working (a ring turns around it; the line under Claude's tabs runs) |
| `lamp-red` | `#FF6369` | needs you (a ping goes out from it) |
| `lamp-green` | `#3DD68C` | done, and Live |
| `lamp-off` | white at 20% | idle: a hollow ring |

Diff, derived from the lamps so the window reads as one system: `add-bg` green at 10%, `add-word` green at 30%,
`add-ink #62E0A2`; `del-bg` red at 10%, `del-word` red at 30%, `del-ink #FF8D92`; `gap-bg` white at 3%;
`remark-band` blue at 13%.

Syntax colors are an IDE's: the dark scheme of JetBrains' IDEs, PyCharm's (keywords orange, strings green, numbers
teal, comments gray, a definition's name blue, decorators olive, `self` and built-in names violet), so each kind of
word reads at a glance. None of them is the diff's red
or green. Settings, General, Code has quiet colors as the other choice: Shiki's **Vesper** theme, mostly white text
with soft peach and mint accents.

The terminal uses the same palette for its ANSI colors and sits on the island's own color (`crates/enkidu-term/src/palette.rs`), so
Claude's output matches the window; its cursor is signal blue.

## States

What a tab, a row or a part of the window can be, and how each is drawn. Nothing else marks a state. Before these
rules there were so many small highlights that it was hard to tell which workspace was on screen; the tab on screen
is marked by its border.

| State | It means | Drawn as |
|---|---|---|
| Rest | nothing | nothing |
| Pointer | the pointer is on it | the `hover` fill |
| Picked | this one is chosen: the arrow keys are on this row, or this tab or this half of a switch is the one shown | the `selected` fill; on the tabs of the top rows and of the terminal the name is ink, semibold |
| On screen | this is where you are | **The workspace tab:** Picked, and the pill's edge in signal blue, 1.5 px (7.91 to 1 against the row). **A row** that lists what is on screen (the file in Changed files and Files, the workspace and the layout in the palette): a 2 px signal-blue bar at its left |
| Keyboard | your typing goes here | a part's edge in `island-edge-focus`; a text field's blue ring; on a workspace tab a 2 px blue ring that stands 2 px inside the tab's edge, so it is never taken for the edge of the tab on screen (`shots/out/tab-keyboard.png`) |
| Window behind | Enkidu is not the front app | the blue edge and the blue bars turn `behind`, and no part has the keyboard's edge |

- **Rest, pointer, picked: each stronger than the one before**, in every control. Picked is 1.23 to 1.27 to 1 against
  pointed at (it was the other way round on the tabs before 14 Nov).
- **Blue is spent once in the tabs row: on the tab on screen.** What waits on you there is not blue: a PR number that
  waits and the count of unsent remarks are ink. The project on screen and the terminal on screen are Picked, without
  blue, so blue boxes don't stack at the top left; the blue under them is the edge of the part you type in.
- **Chosen never rests on brightness that means something else too.** An exited terminal that is on screen keeps a
  readable name, and the word beside it says "exited". In the file tree a changed file has its letter, and its name
  is bright only while it is the file on screen.
- **Three ranks in the top rows:** where you are, then what needs you (red), then the rest, quiet. A done lamp on a
  tab that is not on screen is dimmed to 55%, the badge of a project that is not on screen is gray, the red counts
  have no glow, and Go live's triangle is green only when pointed at.
- **A test keeps it** (`the_marks_can_be_seen`, in `enkidu-ui`): the blue edge, the blue bar and their gray stand at 3
  to 1 or more against the frame, an island, a title band and a dialog; the keyboard's edge at 3 to 1 against the
  frame; picked at 1.2 to 1 or more against pointed at. And the picture test reads its own pictures: the edge of
  the tab on screen is blue in `window.png` and gray in `window-behind.png`, and in `tabs-squint.png`, the top rows
  of five workspaces blurred, the tab on screen still stands at 1.2 to 1 or more against its row.

## Icons

- **File types** (`FileIcon`): a Lucide file glyph per kind (code, JSON, text, config, shell, image, lock, …), colored
  by language in OKLCH at one lightness and chroma (`oklch(0.77 0.13 hue)`), so no type shouts louder than another:
  Python blue, JavaScript yellow, Vue green, HTML orange, CSS violet, SCSS pink, JSON amber, Markdown slate, YAML and
  TOML violet, shell green, SQL teal, images purple; locks, fonts and plain text stay gray. Folders are a quiet steel.
- **What happened to a file** (`FileGlyph`): a letter on its own tint, as git colors it: M tan, A and U green, D red,
  R blue. A folder that holds changes has a tan dot.
- **Projects** (`ProjectBadge`): the first letter in white on the project's own color, lit from above like an app
  icon. The color comes from the project's place in the top row (blue, green, orange, violet, teal, pink, indigo), so
  two projects never share one.
- **Terminals**: Claude's tab carries a filled coral sparkle; a shell, a green terminal.
- **Definitions in Search**: a letter per kind on its own tint (classes blue, functions violet, methods teal…).
- Lucide at 14–15 px, stroke 1.75, everywhere. The layout switch uses its own small window pictures.

## The mark

- **Two wedges standing side by side**, their heads up: you, and the one who works beside you. A wedge is the stroke
  of cuneiform, the writing the story of Gilgamesh and Enkidu was pressed into clay with.
- **The icon** (`bundle/icon.svg`, `bundle/icon.icns`): the first wedge in warm white (`#F3EEE4`), the second in the
  app's own blue (`accent-strong`, `#4F7DFF`), on the window's dark (`#1A2032` down to `#0B0E16`), on macOS's own
  tile (824 px on a 1,024 px canvas, with its soft shadow).
- **The empty screens** draw the same mark in the window's own gray, 70 px wide at most: the two wedges are pressed
  in one after the other, and the second one lights blue (`accent-strong`), as in the icon. It plays once.

## Motion

Tokens: `--ease-out` `cubic-bezier(0.23, 1, 0.32, 1)`, `--ease-in-out` `cubic-bezier(0.77, 0, 0.175, 1)`,
`--ease-pop` (a slight overshoot); `--dur-press` 120 ms, `--dur-quick` 160, `--dur-base` 220, `--dur-slow` 320. Only
transform and opacity move; never ease-in.

| What | How | Why |
|---|---|---|
| A press (`press::pressable`) | scale 0.97, 120 ms | the button heard you |
| Menus, tooltips | grow from their trigger, 160 ms; out in 110; the next tooltip along a toolbar is instant | where they came from |
| Dialogs | rise 8 px and fade, 220 ms, over a fading, lightly blurred scrim | a step above the window |
| The tab pill, segmented thumbs | glide 220 ms to the new choice | which one is on screen |
| Working | a ring turns around the amber lamp; the hairline under Claude's tabs runs amber; "Claude is working" shimmers | Claude is busy |
| Needs you | a ping goes out from the red lamp every 1.7 s | it waits for you |
| A new state | the lamp pops once (460 ms, a small overshoot) | something just changed |
| The changes arrive | the island rises in (220 ms); a panel slides in from the right edge | Claude's first edit, a panel you opened |
| A new changed file | its row flashes signal blue and settles (1.8 s); in a closed folder, the folder's row does | what just turned up |
| Counts, +/− | tick into place (220 ms) when they change, never on first show | more lines changed |
| Badges | pop in with a small overshoot | a new count |
| Toasts | rise from the corner (300 ms), leave to the right (180 ms) | news, and where it went |
| The empty screens | the mark's two wedges are pressed in one after the other, then the second one lights blue | seen rarely, so it may take its time |
| A recording in a report | plays by itself while it is in sight, at its own pace | you asked Claude to show it working |
| The palette, Search | none | opened from the keyboard many times a day |

## Type

- UI: SF Pro through `system-ui`, antialiased. Body 13/18, secondary 12/16, status line 11.5/16, caption 11/14,
  panel title 13/20 semibold, dialog title 15/20 semibold, empty-state title 17/24 semibold. Weights 400, 500 for
  labels, 600 for titles, the Live button and the tab on screen.
- Code: JetBrains Mono, 12/20 in the diff and 12.5 in the terminal, ligatures off so `->` stays `->`. Branch names
  in the status line are mono too. Both sizes are yours to change: ⌘+ and ⌘− resize the text of what has the
  keyboard (the code's while the changes have it, the terminal's while a terminal has it), ⌘0 puts it back, and
  Settings, General, Code has both; a line of code keeps its share of height (5 to 3).
- Numbers that change (counts, +/−) use tabular figures of the UI face.

## Space, size, shape

- 4 px grid. Control heights: 22 (segments), 24 (small), 26 (project tabs), 28 (default), 30
  (workspace tabs), 32 (inputs in dialogs).
- Projects row 38 px (the macOS window buttons are centred in it), tabs row 40 px, island title bands 36 px, list
  rows 36 px (two lines) or 26 px (tree), status line 26 px. Islands sit 8 px apart and 8 px from the window's edges.
- Radii by hierarchy: 5 (tags, kbd, small badges), 8 (buttons, inputs, rows, project tabs), 9 (workspace tabs and
  their pill, rail buttons), 12 (islands, menus, remark cards, toasts), 14 (dialogs, the palette, Search). Live is a
  pill: it is a signal, not a command.
- The row of what is on screen has a 2 px signal-blue bar on its left, so it reads at a glance ("States").
- Buttons: primary is white, lit from above like a key; Open in Claude and Type remarks into Claude are signal blue (`accent`); secondary is a
  hairline on a faint fill; all answer a press.

## Live

A play button at the right of the tabs row, one fixed width so nothing beside it moves:

| State | Looks like |
|---|---|
| Off | ▶ Go live: a hairline pill; the triangle turns green when pointed at |
| Starting / Stopping | a spinner and "Starting…" / "Stopping…" |
| Live | a green pill with three playing bars and "Live" |
| Live, pointed at | ■ Stop, same green: clicking stops it |

On a tab and in the palette, Live is the small green pill with the bars (a compact tab keeps only the bars), and the
status line says "Live in shop's dev environment". One workspace per project can be Live; going live elsewhere stops
the old one first, and the tooltip says so.

What it does: the dev environment runs from the project's main folder, so **Go live copies the workspace's changes
into the main folder**: the files in its changes list (modified, new and deleted, as `git status` shows them), and it
keeps copying each new change while it is Live. A file that stops being a change (Claude committed it, or it was put
back) gets the main folder's own version again. **Stop puts the main folder back** as it had it, its own edits
included. Never copied: ignored files, submodules, and the project's "Files to copy" (`.env` and the like, which are
the main folder's). Nothing in the main folder is lost: before Live first writes over a file, the main folder's own
version is saved, and a file changed in the main folder while Live is set aside (a toast says where) before Live
touches it again. The project's optional commands run after the copy and after putting it back (to restart a server).
The toast on Go live says what went where: "Copied 9 files with its changes into ~/code/shop.example; new
changes follow while it is Live."

## Layout

```
Changes above the terminal (default)
┌────────────────────────────────────────────────────────────────────────────────┐
│● ● ●  S shop ⌄  E enkidu ❶  ⊕                                                  ⌘ ⚙ │ 38 projects
├────────────────────────────────────────────────────────────────────────────────┤
│ ◌ stock-fix 5 │ ● sh-21309 Needs you │ ● pr-2175 ▮▮ Live │ +     ◫ ⬓ ▭ ▶ Go live ⋯ │ 40 tabs
│ ╭──────────────────────────────────────────────╮ ╭───────────────────╮ ╭──╮     │
│ │ Changes                          ▤ ◫ ⤢       │ │ Changed files     │ │▤⁵│     │
│ │ ▸ M stock.py  src/orders   1 remark  +13 −6  │ │ ⌄ src/orders 2    │ │⌸ │     │
│ │ Before               │ After                 │ │   stock.py  +13 M │ ╰──╯     │
│ │ diff, every file stacked, remarks on lines   │ │   test_stock.py A │          │
│ │ ● 1 remark not sent yet   Type 1 remark into │ │                   │          │
│ ╰──────────────────────────────────────────────╯ │                   │          │
│ ╭──────────────────────────────────────────────╮ │                   │          │
│ │ Claude  +                                    │ │                   │          │
│ │ terminal                                     │ │                   │          │
│ ╰──────────────────────────────────────────────╯ ╰───────────────────╯          │
│ ⎇ fix/stock-double-count from development │ ● Claude is working    5 changed files │ 26 status
└────────────────────────────────────────────────────────────────────────────────┘
```

- **Projects on top, their workspaces under them.** The projects row is the app's level: each project by its name
  (its workspaces are the tabs under it, so they aren't counted); the one on screen opens its menu (new workspace,
  settings, Finder, remove), another one is a click away and shows a red count when one of its workspaces needs you.
  There is no search field: ⇧⇧, ⌘P, ⌘O, ⇧⌘F and the palette open Search. The tabs row is the workspace's level: the
  project's workspaces as tabs (the signal, the name, then what it waits for: "Needs you", Live, unsent remarks or
  the number of changed files), New workspace, and on the right what acts on the workspace on screen: the layout,
  Live and its menu. ⌘1–9 are the tabs, ⌃⇥ / ⌃⇧⇥ (or ⌘⇧] / ⌘⇧[) the next and previous tab, ← and → move between
  tabs from the keyboard. The palette (⌘K) reaches every workspace of every project.
- **Compact when it has to be.** With little room, tabs drop their extra words first (Live keeps its bars, counts
  keep their numbers, "Needs you" stays), then shrink like a browser's; when even that doesn't fit, the strip
  scrolls and fades at the cut edge. With no workspaces yet there is no tab row: the empty screen under the
  projects says what a workspace is and has the one New workspace button (and with no project, the one Add project).
  The workspace's menu (the … at the right of the row): its name, Show in Finder, Copy path, Copy branch name,
  Project settings, then Hide workspace and Remove workspace…, which asks first.
- **Your own order, and workspaces put away**.
  - **Order:** drag a tab along the row and let it go on the tab whose place it should take. While you drag, the
    tab you hold follows the pointer, its place in the row dims, and a blue line stands at the edge of the tab under
    the pointer where yours will land (no tooltip comes while a tab is held). The order is the project's and is
    kept with where you were (the app's `view.json`, beside the workspace you had in each project); ⌘1–9, ⌃⇥ and
    the palette follow it. A new workspace joins at the end.
  - **Hide:** "Hide workspace" in the workspace's menu (the … at the right, for the workspace on screen), above
    Remove workspace…; a right-click on any tab opens the same menu for that tab's workspace, so one can be put away
    without going to it. Its tab goes, and nothing else: its folder, its branch, its conversation and whatever runs
    in its terminals stay as they are (nothing is stopped). Was it on screen, the next tab shows. A hidden workspace
    is quiet: it is not in the project's red count, ⌘1–9 and ⌃⇥ pass it by, and at the next launch its Claude does
    not come back until the workspace does.
  - **Hidden workspaces** stand behind a small button right after the tabs, an eye with a line through it and their
    number, there only while the project has any. Its menu, "Hidden workspaces", lists them (its lamp, its name,
    and since when: "since Mon 2 Nov"); a click brings one back to its place in the order and on screen. What is
    hidden is kept with where you were, like the order. Going to a hidden workspace any other way brings
    it back too: the palette lists it with "hidden", and a card's or a run's "Open …" shows it. With every
    workspace of a project hidden the row holds that button and New workspace, and the screen under it says so.
- **The main folder as a workspace**. The New workspace dialog asks first **where Claude
  works**: "A new workspace" (its own checkout on its own branch: the name and the branch, as before) or "The main
  folder". For the main folder the dialog shows the folder and one sentence (Claude works in the project's own
  folder, on the branch it is on; nothing is made or copied there; removing the workspace later only forgets it),
  and its button says "Open the main folder". Its tab is named `main-folder` and is a workspace like any other (its
  Claude, its terminals, its changes; it can be hidden and moved), with two differences. Nothing was made for it, so
  Remove workspace… only forgets it and the folder stays. And it can't go Live (Live copies a workspace's changes
  into the main folder), so there is no Go live beside it. Asked for again, the main folder that is open already
  comes on screen. The screen of a project without workspaces names both ways. On the main folder's side no field
  is shown, so the dialog itself holds the keyboard: Enter opens the folder, Esc closes the dialog.
- **The status line** says in words what the tabs say in signs, for the workspace on screen: its branch and where it
  started, "Claude is working" (or idle, needs you, done) with its lamp, Live, the remarks not sent yet, the changed
  files with +/− (nothing when there are none), and the folder (a click copies the path).
- **The lamp tells the truth.** Claude's hooks set working, needs you and done. Claude Code runs no hook when you
  interrupt it, so Esc or Ctrl+C in Claude's terminal, then Claude redrawing and going quiet for 1.5 s, sets the
  workspace back to idle; a key that interrupted nothing changes nothing.
- **Three layouts**, switched from the tabs row (◫ ⬓ ▭), the palette, Settings or the keyboard: changes above the
  terminal (⌥⌘L toggles with beside), changes beside it, or the terminal alone (⌥⌘B). ⇧⌘E gives the changes the
  whole workspace. The space between two islands drags to resize and remembers; double-click it to reset.
- **The changes are what isn't committed yet**: every file that differs from HEAD (staged or not) and every new file,
  never a comparison with development or another base. While a merge is on (stopped on a conflict, or `--no-commit`),
  git stages everything the merge brings in, so to git it is all uncommitted (it can be hundreds of files when a main
  branch is merged into a PR branch); none of it is your work, so the list compares each file with what
  the merge makes of it on its own (`git merge-tree`): it shows the conflicts (against HEAD, markers and all) and what
  you or Claude change on top, and Live copies only those. Once the merge is committed, HEAD is the base again.
- **Nothing to review, nothing shown.** With no changes, no remarks and no file open, Claude's terminal has the whole
  workspace, whatever the layout; the first edit (or a remark, or a file opened) brings the changes back where the
  layout puts them. The Changed files panel, if open, keeps its title and nothing under it. The window opens with
  every workspace's changes as last seen (git's fresh list follows), so the layout doesn't jump after it shows.
- **The right edge is a tool bar** (like an IDE's): Changed files (▤, with their number) and Files (⌸). Each opens
  its panel beside the bar, full height; a second click, or ⌥⌘1 / ⌥⌘2, closes it. The panel's width drags and
  is remembered. Changed files is a tree of the folders the changed files are in, as an IDE shows a commit: each
  folder says how many changed files it holds, a folder that only holds one folder shares its row ("src/orders"), and
  in every folder its folders come first, then its files, both by name. The diffs are stacked in that same order, so
  the mark of the file on screen moves down the tree as you scroll them. Every folder is open at first; a click closes
  one, and it stays closed for that workspace while the app runs. Over the tree, at the right, stand "Expand all" and
  "Collapse all" (there while the tree has a folder; the one with nothing to do is dimmed): Collapse all closes every
  folder, the ones inside closed folders too, so the top folders are left with their counts, and a folder opened after
  that shows its own folders closed. At the left of that row stands the filter Find in files has: "All files", or one
  kind of file such as `*.py`, with how many changed files each kind has (there while the changed files are of more
  than one kind). A picked kind keeps its files in the tree, with the folders that hold them, and the button turns
  signal blue; the band still counts every changed file, and the diffs stay whole. A file's row is one line: its name,
  its remarks, its lines and its letter (its tooltip has the whole path). Click one to scroll its diff into view; the
  keyboard goes there with you, so the arrows scroll it and ⌘+ and ⌘− resize its code. The file on screen is
  marked, and while its folder is closed that folder is; a file that turns up lights up once, or its closed folder
  does. Under the tree stand the files that have remarks but no changes, by path. Files is the workspace as git sees
  it (tracked and new files, the submodules' too; ignored ones left out): folders first, single-folder chains on one
  row ("a/b/c"), an amber dot on folders that hold changes, the change letter on changed files, and a filter that
  ranks file names first. Clicking a file opens it where the diffs are, with syntax colors; a changed file offers
  "Show changes", and Escape goes back.
- **What differs, in an opened file.** The lines that differ from the last commit carry a stripe beside their number,
  as in PyCharm: a new line, a changed one, and a small mark where lines went. The colors are the code scheme's own
  (PyCharm's green, blue and gray with "Like an IDE"; with "Quiet" the tones of the file badges). They come from the
  diff the Changes list already reads, and change in the same frame as the text when Claude writes the file. A file
  that is all new has none: every line would have one, and its badge says it.
- **Remarks on any line.** In the diff and in an opened file alike, pressing a line number writes a remark under that
  line; dragging over line numbers (or Shift-clicking) makes it a range. A diff's file header opens the whole file
  (📄), to leave remarks on lines the diff doesn't show. Remarks on files without changes are listed under the diffs,
  by file, with "Open file". They all go to Claude together, from the blue bar at the foot of the changes or of the
  opened file: "Review remarks on the code in this workspace", then each remark with its file:line and the lines it
  is about.
- **Search**, like PyCharm's Search Everywhere: one popup with three tabs, opened from the keyboard or the palette
  (no field on screen). Files (⇧⌘O or ⌘P): names, the changed and recently opened files before you type. Symbols (⌘O):
  classes, functions and methods by fuzzy name ("slv" finds StockLevelView), with a kind letter, the class a method
  belongs to, and file:line. Text (⇧⌘F) is laid out as PyCharm's Find in Files: one row for each line with a match,
  in the code's colors with what was found marked, its file's name and line number at the right, the line that
  defines the searched name first; beside the field a filter for the kind of file ("All files", or one kind such as
  `*.py`, with how many files each has) and the match case, whole words and regex toggles (⌥⌘C, ⌥⌘W, ⌥⌘R); under the
  rows the file around the picked line, with the marks of what differs. It reads the files git tracks, the new
  (unversioned) ones and the submodules', never what git ignores. ⇧⇧ (Shift tapped twice) opens the tab used last; ⇥
  switches tabs; ↩ opens the result at its line, the line marked and centred. Only two clean taps of Shift count, so
  typing capitals never opens it. How it stays fast on a huge repo: SEARCH.md.
- **Go to definition.** With ⌘ held, a name its language's server finds in the workspace is a link (signal blue,
  underlined), in an opened file and in a diff's side as it is now. ⌘-click goes to its definition: the file opens
  at its line. ⌘[ goes back to where the click was made and ⌘] forward again, and the file's first button says
  where back is ("Back to views.py"). One place is gone to. Several places open a small list, each row with what it
  is, its class, its folder and its file with the line; ↑ ↓ ↩ or a click picks one.
  Who answers: the language's own server, and nobody else. The app has no rules of its own for this. For Python it
  is Astral's `ty`, carried inside the app. It knows what a variable is: a
  variable is a link to the line that sets it, and a method of a variable goes to its class's. One server runs
  for each workspace asked lately (three at most), started when the workspace's Python code comes on screen and
  stopped after ten minutes without a question. A question gets 150 ms; while the server is starting or busy, the
  name the pointer rests on is asked about again until it answers. What the server doesn't find is no link, and
  neither is a place in a library outside the workspace's folder. JavaScript, TypeScript and Vue have no server
  in the app yet: no links there until theirs is added.

## The Work page

Your tickets as a board, the way Jira shows them: a column per Jira status, a card per ticket, with its PRs and the
Claude sessions behind them (WORK-INDEX.md). A card says only what you read at a glance; clicking it opens the
ticket in front of the board, with what the task is, what needs you, and how it stands: Jira's status, its pull
request and the PR's checks and review, each always in the same place.

What it takes from the tools that do this best:

| From | What | Here |
|---|---|---|
| Jira's board | a column per status in the workflow's order; a column can collapse; a card has the type icon, the key and the priority | the columns, a card's first line |
| Jira's ticket view | the story on the left (description, activity), the facts on the right: status first, then the development panel ("1 pull request OPEN") | the card in front of the board |
| Linear's board | quiet dark cards: a title, one line of state, small colored glyphs; a status icon per column | the cards; the column rings |
| GitHub's PR list | the review state in words, the checks as one mark | a PR's state in a word or two on its card |
| Vercel's deployments | a status is a dot and one word | the checks; the source lamps |

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│● ● ●  ▣ Work ❹   S shop  E enkidu  ⊕                                                    ⌘ ⚙  │ projects
├──────────────────────────────────────────────────────────────────────────────────────────────┤
│ ● Needs you 4                                               ⌕ Filter  /   ● GitHub ● Jira ⛨ ↻ │ 40
│ ╭ ○ New 1 ─╮╭ ⏸ On Hold 1 ──────╮╭ ◔ In Progress 3 ────╮╭ ◑ Code Review 3 ──╮╭ ✓ Done 1 ───╮ │
│ │╭────────╮││╭─────────────────╮││╭──────────────────╮││╭──────────────────╮││╭───────────╮│ │
│ ││☑ 12955 ││││☑ SH-21475 ⌄     │││▣ SH-21309 ⌃   (4)│││🐞 SH-21533 ⇈ (2)│││▣ SH-21030 ││ │
│ ││Outlet  ││││Sales report per │││Export orders to  │││Stock counts twice│││Warranty   ││ │
│ ││groups… ││││outlet           │││the accounting CSV│││after a return    │││labels on… ││ │
│ │╰────────╯││╰─────────────────╯││⇡ #2414 Checks    │││⇡ #2413 Checks run│││⇡ #2312    ││ │
│ │          ││                   ││  failing         │││⇡ #2395 Changes   │││  Merged   ││ │
│ │          ││                   ││✳ sh-21309-export ●││  requested       ││╰───────────╯│ │
│ ╰──────────╯╰───────────────────╯╰────────────────────╯╰────────────────────╯╰─────────────╯ │
│ ● GitHub read 2 min ago as sam-lee │ ● Jira read 2 min ago as Sam Lee    ←→↑↓ move ↩ open … │ status
└──────────────────────────────────────────────────────────────────────────────────────────────┘

The card in front of the board:
╭───────────────────────────────────────────────────────────────────────────────────────────╮
│ ▣ Story  SH-21309                                                      Open in Jira ↗   ✕ │
│ Export orders to the accounting CSV                                                       │
├──────────────────────────────────────────────────────────┬────────────────────────────────┤
│ The task                                                 │ ╭ Jira ──────────────────────╮ │
│ Accounting imports the orders of a month as a CSV…       │ │ Status    [In Progress]    │ │
│ What to build                                            │ │ Type      ▣ Story          │ │
│  • An Export to CSV action on the order list…            │ │ Priority  ⌃ High           │ │
│ ░░ fading ░░  Show the whole description                 │ ╰────────────────────────────╯ │
│                                                          │ ╭ Pull request ──── GitHub ↗ ╮ │
│ Needs you 4                                              │ │ ⇡ #2414 [Open]             │ │
│ ╭──────────────────────────────────────────────────────╮ │ │ ⎇ feature/SH-21309… → dev  │ │
│ │ ⍰ jonas-qa asked a question                   14 min │ │ │ Checks  ● Failing: tests   │ │
│ │ │ Does the export include orders that were cancel… │ │ │ Review  Waiting for review │ │
│ │ [✳ Open in Claude] [⏰] [✓]                          │ │ ╰────────────────────────────╯ │
│ ╰──────────────────────────────────────────────────────╯ │ ╭ Claude ────────────────────╮ │
│ Activity                                                 │ │ ✳ sh-21309-export  ●  Open │ │
╰──────────────────────────────────────────────────────────┴────────────────────────────────╯
```

- **Where it lives.** "Work" is the first tab of the projects row (it spans projects), with a red count when
  something waits on you and a red mark instead when a source can't be read. ⌘I opens it and goes back; so does Esc.
  It takes the place of the workspace's tabs and islands; the projects row and the status line stay.
- **The header row** (40 px, where the workspace tabs were): **Tasks | Hidden 4** once you have hidden a task (the
  board, or only the tasks you hid; Hidden has a red dot when something waits on you there); "Needs you" with its
  count, on the board (cards with something waiting on you; on, it shows only those; with nothing waiting it isn't
  there); the filter field (/) for keys, titles, branches and names, in the view you are in; each source's lamp
  (green read, amber reading, red can't read, hollow off) with its age on hover; Check connections; Refresh. While a
  read runs, the hairline under the row runs amber, like Claude's working line. No project chips: the projects are
  the tabs above, and the board holds every project's tasks. Needs you is
  remembered; the page always opens on the board.
- **A broken source says so, first.** Above the board, one line per source that can't be read: what is wrong and what
  to do ("Jira can't be read: twg isn't signed in. Run `twg login --force`."), and how old the data under it is.
  Cards keep their place; one whose waiting items come from that source is dimmed, with the time of its last good read.
- **A column per Jira status** of the cards on the board, in the workflow's order, worked out from the statuses' own
  words and categories (Jira's to do, in progress and done): On Hold first, at the far left (parked work out of
  the way), then Backlog, New, Analysed, Ready for Dev, Planned, In Progress, Code Review, QA, Release; Done. Each column is an island: its ring, its name as Jira writes it,
  its count. The ring is hollow for to do, fills further as the work moves right (blue, review violet, QA amber,
  release teal: the status pill's hues), pauses on hold and turns into a green check when done. The board is
  everything read: the backlog's statuses are columns like the rest, and no column folds; what you don't want to see,
  you hide. A status still to come stands before the one it leads to ("To be analysed", then "Analysed"). Where the
  board goes on past the window, that edge fades, so a cut card reads as "scroll".
- **A card per ticket**, what needs you first in each column, then the latest activity. Its first line: the type as
  Jira colors it (bug red, task blue, story green, sub-task, epic violet), the key in mono, a priority mark only when
  it isn't Medium (up in orange or red, down in blue), and on the right a badge counting what waits on you (red when
  something blocks: failing checks, changes requested, a conflict, back to you; blue otherwise; the words on hover).
  Then the title, up to three lines. Then a line per PR (two, then "and 1 more"): its state glyph, its number and its
  state in a word or two, the most pressing first: Conflicts, Checks failing, Changes requested (red); Draft; Checks
  running (amber); Approved (green); Waiting for review; Merged (violet). Last, the Claude in an Enkidu workspace
  on it, with its lamp. Nothing else: the rest is one click away.
- **A PR whose branch and title name no ticket** is a card of its own (repo#number, its title, its state), in the
  column its state matches: a draft in progress, an open PR in review, a merged one in done, named as Jira names that
  status when one of your tickets has it ("Code Review"), else In Progress, In Review and Done: the same column in
  every view.
- **Pick tasks, hide them, unhide them.** A round box sits on a card's corner on hover (and on every card while you
  pick); a click on it, ⌘ with a click, or X on the card the keyboard is on picks it; ⇧ with a click picks every card
  from the last one picked (down each column, then on to the next, as in Finder); while you pick, a click on a card
  picks it too. A bar rises at the bottom: "3 cards picked", Hide (on the board) or Unhide (in Hidden), Clear (Esc).
  Hidden tasks leave the board. **Hidden** in the header shows only them, in their statuses' columns, where they are
  picked and unhidden; Tasks or Esc goes back to the board, and so does unhiding the last one. They count neither in
  Needs you nor on the Work tab, but Hidden has a red dot when something waits on you there, so nothing is lost by
  hiding. The card in front of the board has Hide (or Unhide) too, and closes after it. Hidden tasks live in the
  app's database (a restart, or a reinstall, keeps them), and each card's timeline says when.
- **The card in front of the board** (a dialog, 1040 px): the type, the key, Hide, Open in Jira, then the title, then
  tabs: **Overview**, a tab per open PR (**PR #2414**), **Activity** (when each has something). Overview is the story:
  **The task** (Jira's description drawn in the page's own type: headings, lists, checklists, code, quotes, panels,
  tables and links; a long one shows its start, fading, and Show the whole description, so what needs you is in view
  too) and **Needs you** (each item as a sentence a person would say, "mira-k left a remark on export.py, line 48", the
  words quoted under it, a box that keeps it in the message to Claude or leaves it out, a field for how Claude should
  handle it, and Open, Snooze and Done). A **PR's tab** says what is going on there, all from the same read: who
  opened it and when, its size (+412 −38 in 9 files), commits and labels; **What the PR says** (its description,
  GitHub's Markdown drawn as text: HTML comments from the template dropped, images named, never fetched); **Checks**
  (counted by how they stand; the failing and running ones listed, the ones that passed behind a click); **Reviews**
  (each reviewer's latest review in words, Approved, Asked for changes, Commented, and who is asked and hasn't yet);
  **Review threads** (the open ones, each with its file and line, who started it and what they said, how many replies
  and the latest; the resolved ones behind a click); the **Conversation**; the **Latest commits**. On the right,
  always in this order: **Jira** (the status as a large pill, type, priority, assignee, updated), **Pull request** (a
  box per PR: number and Open, Draft, Merged or Closed; the title; branch to base; Checks; Review; a merge conflict;
  open threads; the GitHub link), or "No pull request yet.", and **Claude**: the choice of which Claude the message
  goes to. Each Claude linked to the card is a row you can pick (its workspace or where it ran, its lamp, its strongest
  link in words, "Created it, 2 h ago", and Open for one in a workspace), the likeliest first, then **New Claude** (a
  new conversation in the card's workspace, or a new workspace). The workspace on the card's branch is always there,
  also before a hook linked its conversation. Picked unless you pick another: the Claude whose workspace is on the
  card's branch; else one you or Claude linked to it (`enkidu link`); else one that made, pushed or committed to the PR and
  isn't busy with another ticket (its workspace on a branch named for another key); else New Claude. Being named in a
  prompt is never enough. Your pick is kept with the card's
  draft and the board card shows that Claude; the "Tell Claude" line at the bottom says where the message goes, and a
  click on it brings the box into view. With nothing for the tabs, only the facts, in a narrow dialog. A PR without a
  ticket has its GitHub link once, in its box.
- **Tell Claude** is the bottom of the card, always in view: where it goes ("goes into the prompt of the Claude in
  sh-21309-export; you press Enter there"), a field for what Claude should do about the task, and what goes with your
  words, as chips: the ticket, its open PRs, "3 of the 4 things that need you". See the message shows it whole.
  **Open in Claude** (⌘↩, from any of its fields) types one message into that Claude's prompt and takes you there,
  the keyboard on it: the card's key and title, the ticket's and the PRs' links, your words, then each item you kept,
  quoted, with your instruction for it, or the usual one ("Find out why and fix it.", "Draft an answer."). Nothing is
  ever sent for you: you read it in Claude's prompt and press Enter. With no
  Claude on the card yet, the button is **Start workspace** (below). What you write is kept until it goes, also
  across a restart; then it is cleared.
- **Keys:** on the board, the arrows (or h j k l) move from card to card, ↩ opens one, X picks it, / filters, Esc
  lets go of the picked cards, then leaves Hidden, then goes back (⌘I too). In front of a card, j/k (↑/↓) move through what needs you, x
  leaves it out of the message (or puts it back), n writes its instruction, s writes to Claude, ⌘↩ opens it in Claude, e marks it
  done, z snoozes it for a day, ↩ opens where it was said, Esc leaves a field, then closes. The status line shows the
  keys that work right now. Moving with the keyboard never animates.
- **Motion with a job:** the card rises in like every dialog; a card that moved to another column glows once, so you
  see it move; counts tick; a PR's dot pops when its checks flip; the running line while reading. Reduced motion
  keeps fades only.
- **Nothing about nothing:** no column without cards, no chip with a zero, no empty half of the dialog. The first run,
  with nothing read yet, says what is connected and checks; that is an action, not an empty state.
- **Descriptions are read once per change:** a ticket's description, type and priority are read when it is new or its
  `updated` changed, 25 at a time, and kept. (Asking for descriptions in every search makes its answer nine times
  bigger: 477 KB instead of 53 KB for 44 tickets.)
- **Where a message goes** (the one accent action of a card, at the bottom of it; the toast and the timeline name the
  place), never with Enter: to the Claude picked in the card's Claude box. Typed into it when it is the conversation a
  workspace's Claude is in; when Claude isn't running there, Claude starts and gets it typed in once its session has
  started (its SessionStart hook, then a moment of quiet, so never into its folder-trust question). A conversation that
  isn't any workspace's now (an older one of a workspace, or one from outside Enkidu) is resumed in its own
  workspace or the card's (**Start Claude**), else brought into a new workspace on the card's branch, in its own
  account. **New Claude** starts a new conversation in the card's workspace (**Start Claude**). Moving a workspace to
  another conversation closes its Claude only when it isn't working or waiting for you (else it says so and does
  nothing); the conversation it was in stays saved, and stays on the card to pick. Sending links that Claude to the
  ticket and its PRs ("linked by you"), so the card goes there next time. **Start workspace**, when no Claude has worked on the card:
  a workspace named for the ticket, `sh-21000-price-lists-keep-their-order`, on its open PR's branch, else on a new
  branch `feature/SH-21000-price-lists-keep-their-order` from the project's base (a branch already there is checked
  out; a workspace already on it is the one used). A branch checked out in another folder (git checks a branch out
  once) asks, in the card, what to do: **Open that folder as a workspace** (as it is: nothing copied, no setup script,
  and Remove only forgets it) or **a new branch from it** (its name filled in, `feature/…-2`, starting where that
  branch is, its own unpushed commits too); the New workspace dialog asks the same. The steps show beside the button
  while it is made ("Running the setup script"); Claude starts there before the workspace comes on screen, with the
  message to type in. Only a card
  with no project to start in puts the message on the clipboard. The message always ends by asking Claude to draft
  replies, never to post them. The failing-checks workflow types an item the same way, by itself, only into a Claude
  that worked on the PR, and never takes you anywhere. Review remarks go the same way: Type 3 remarks into Claude puts
  them in Claude's prompt and gives Claude the keyboard.
- **Check connections** is the shield next to Refresh: a dialog that reads both sources fresh, then asks `gh` and
  `twg` directly and compares. One card per source and project: a green check, "matches" and what was compared, or a
  red ×, "can't be read" with the reason, or each difference in one line ("#2414 title: the index has …, GitHub says
  …"). Every ticket on the board is compared too: its status (its column), type, priority and description (a
  description that differs says how long each is and how it starts); and every open PR's tab: its size, author,
  labels, who is asked to review, each reviewer's latest review and every check by name. The running line runs while
  it checks; Check again repeats it. `enkidu doctor` prints the same in a terminal.
- **Settings → Claude → Claude outside Enkidu:** one row per Claude folder (its settings.json in mono) with one
  button, Add hooks or Remove, and "Added" in green once there. "Show what it adds" unfolds the exact JSON. Nothing is
  written until the button is pressed.
- **An item's actions fit one line:** Open, Snooze and Done as quiet squares whose tooltips give their words and keys.
  Pressing one never moves the keyboard's place; a plain click on an item puts the keyboard there.
- **Settings → a project → Work:** what is read for that project: a switch for its PRs on GitHub, the gh account
  (empty: the first that can read the repo), Jira's project keys (empty: Jira isn't read; saved in capitals), the
  statuses that mean a ticket came back to you, the checks that never count (one a line), and the minutes between
  reads (1 to 240). Saved with the page.
- **Settings → a project → Workflows:** two switches, each with one line on what it does, both off until turned on.
  The morning summary has its time ("At", 24 h) and Run now beside it. Under them, the latest runs: a lamp (green
  done, gray skipped, red failed, amber while running, its words shimmering like "Claude is working"), the workflow's
  name, what happened or why not (code as chips), and the age. A summary that ends says so in a toast.
- **Code in sentences is a chip**, never backticks: a small mono pill that doesn't break across lines.

### A Jira board, the message as it goes, Claude's reports, workflows

What it is for: picking the Jira board whose tasks are shown, reading the whole message before it goes to Claude,
starting a new workspace from a card, running tasks one after another at night with Live on, and seeing on a task
the pictures Claude took when it finished.

- **The board menu** is the first thing in the header: a kanban glyph and "All tasks", or the board's name (the glyph
  blue then). It opens a menu: All tasks, then "Jira boards", each with "sprint" (scrum) or "kanban" on its right.
  Picked, only that board's tickets stay, exactly as its own page in Jira shows them (a scrum board: your tickets in
  its running sprint, in the statuses of its columns); a PR that names no ticket is on no board. The pick is kept in
  the app's database. While the board's tickets are read (a second or two) the page says nothing; after, a board with
  none of yours says so in one line.
- **The Claude box** lists, under the Claudes linked to the card: New Claude (a new conversation in the card's
  workspace), New workspace (only when the card has a workspace: a fresh one for it, `sh-21309-export-2`), and
  "Another workspace's Claude…", a menu of every other workspace's Claude in the project ("Send it to the Claude in").
  One you pick that way shows as "Picked by you". A link that is wrong has an unlink glyph that goes red on hover
  ("Not this task's Claude: unlink it"); the link stays gone (the index doesn't make it again).
- **The message as it goes:** "See the message" shows the whole message, word for word, in an editable field (mono).
  Edit it and your text is the message: the field for your words steps aside, the link turns into "Undo my edits",
  and the parts on the left no longer change it. Sent, the edit goes with the rest of the draft.
- **Claude's report** (`enkidu report done|review|blocked "…" --image FILE`): on the card, the latest one's state at the
  card's foot: a green check "Done", an amber eye "Review", a red mark "Stuck", with the number of screenshots. In the
  card, it comes first: "Claude's report", a tinted pill with the words (Finished, nothing needs you / Finished, wants
  your review / Stuck, needs you), when and which workspace's Claude (a link to it), Claude's words as Markdown, and
  its screenshots as thumbnails, one big or a grid of two or three. A thumbnail opens the screenshot as large as the
  window (← → go through the others, Esc closes it, not the card), whole and at its own size, never shrunk below
  it. Each thumbnail says what kind of file it is in its top left corner, in a small dark label: PNG, JPG, GIF,
  WEBP. A recording among them (a GIF) plays by itself, small as well as large, at its own size however long it is,
  read from its file a picture at a time. Small, it plays only while it is in sight: not scrolled away, not behind
  a screenshot shown large; with "Reduce motion" it stands still on its first picture that shows something (many
  start on an empty page). Earlier reports are one click away.
- **Workflows:** picked cards get "Workflow…" in the pick bar. It opens a dialog in two parts, **a chain and the step
  you picked in it**.
  - **Left, the chain** (360 px): "Put them in" (a new workflow or one you have; only when you have one) and the
    name, then the workflow from top to bottom, on a rail. **How it starts**: "Starts tomorrow at 02:00" (or "Kept
    until you start it"), and under it "Up to 60 min a task. Live follows." **Each task**, two lines: its number on
    the rail, its key and title, and at the right the time it starts at the latest ("02:00" for the first, "by
    03:00" after it; nothing while the workflow has no time); under them its kind of work, its minutes and where it
    runs in a few words ("Answer the review, 45 min, its Claude"; a new conversation; a new workspace). **How it
    ends**: "Ends by 06:45 at the latest" ("Takes 4 h 45 min at the most" without a time), and "Live goes back to
    the workspace it was on." Under the chain: New task.
  - **Right, the step that is picked** (in the chain it is pressed in, with the blue bar of what is on screen). The
    start is "When it runs, and what every task gets", set once for the whole workflow: "Starts at" (its switch; 24
    h; "tomorrow at 02:00" beside it, "kept until you start it" when it is off), "A task gets up to 60 min, unless
    it says otherwise", "Live follows the task that runs", "For every task (optional)", and the line on what is
    different about a workflow: it sends each message with Enter when its turn comes, and keeps the Mac awake (leave
    it open and on power). A task is its title, key and "task 3 of 5" with ↑ ↓ × at the right, then its own setup
    (next section). The dialog opens on the start, the keyboard in the name; a task made with New task is the step
    shown; a task taken out hands the step to the one that takes its place.
  - **The foot** says the plan in one line, "5 tasks, one after another. At most 4 h 45 min, done by 06:45." (in
    amber, what is still missing, while something is), then Cancel, Run now, and the white button ("Start tomorrow
    at 02:00", or "Keep it"). The dialog is 1040 px wide and keeps one height (820 px, or 78% of a smaller window),
    so it stays still as you go from step to step; the chain and the step scroll on their own.

  The header gets "Workflows" (a running line beside it while one runs), which opens their page (next section). A
  card in a workflow that is due shows its start time ("02:00") by its key; the task that runs shows "Now".
- **What a run does:** at its time, task after task: its workspace (made if new), the card linked to that Claude, Live
  moved to it, its message typed and sent. It ends when Claude reports, or at its minutes (then the next starts; its
  Claude isn't stopped). Claude's Stop hook asks once for a report Claude didn't make (Claude goes on with it, as
  Claude Code does with a stop hook's answer); a Claude that stays quiet is asked once more in its prompt, then the
  task closes as "Stopped without a report". A question Claude waits on for 10 minutes ends its task as "Asked you
  something". At the end, the main folder goes back to the workspace that was Live before. A start time that passed
  more than 30 minutes ago while Enkidu wasn't running is "Missed", never started late.
- **A task whose card has a workspace** runs there by default ("Runs in: The Claude in …"): its Claude gets the message
  in the conversation it is in, with what it knew; one that isn't running starts (resuming that conversation) and
  gets it once up; one waiting for your answer gets nothing typed in (the task says so, the next starts). Picked
  "A new workspace" instead, it gets its own, on a new branch starting where the ticket's branch is (git checks a
  branch out in one folder only), as a new tab; the old workspace and its uncommitted changes stay as they are. The
  workspace made for a task is the task's from then on: running the workflow again goes on there.
- **Nothing is committed or pushed by a workflow** ("when I wake up, I want to see the changes in the diff"): every
  message says so after the instruction, and Enkidu's PreToolUse hook for Bash refuses commands that make or send
  commits (`git commit`, `push`, `merge`, `rebase`, `cherry-pick`, `pull`, `am`, `revert`, `gh pr create`/`merge`)
  while a workflow's task runs in that workspace, with the reason for Claude. A task is marked running before its
  message is typed, so the hook knows it from Claude's first step.

### Workflows: each task its own, and a page of their runs

What it is for: words of its own for each task, or another workspace for it; a task that comes back; On Hold as the
first column at the left; seeing how a run goes and how it ended; Claude reaching the workflow with messages and
pictures; a page with the room of the whole window; and a page of the workflows that ran, each with its tasks'
results.

- **The board:** On Hold is the first column, at the far left.
- **Each task is its own piece of work.** In the workflow's dialog every task (the step picked in the chain) has "Do"
  (a kind of work), "for up to N min" (its own minutes, 5 to 480; the workflow's otherwise), "Runs in" (the Claude
  in its workspace, **a new conversation in its workspace**, or a new workspace), "What Claude is told" (its own
  words: they start as the kind's words, yours to change), "With what waits on it: …" (the card's QA comments,
  review remarks and failing checks go into its message), and its
  whole message, editable. The kinds: Start it, Carry on, Fix what came back, Answer the review, Fix the failing
  checks, Investigate only (no code changed), Your own words. The kind is picked from the card: a ticket QA sent back
  (or a comment that mentions you) is "Fix what came back"; review remarks or a question, "Answer the review"; failing
  checks, "Fix the failing checks"; an open PR or a workspace, "Carry on"; else "Start it". "Use these words for every
  'Fix what came back'" keeps your words as that kind's default, in the app's database.
- **The message, in order:** the ticket's and PR's links; the task's own words; "What waits on it:" (numbered, each
  with its link); "For every task of this workflow: …"; the no-commit line; then the workflow's line (its minutes,
  Live) and how to tell Enkidu how it goes.
- **Claude's CLI during a run:** `enkidu update "…" --image FILE` says how it goes (shown under the task; it ends nothing),
  `enkidu report done|review|blocked "…" --image FILE` ends the task, and `enkidu workflow` tells Claude the workflow it is in:
  its task (n of m, its kind), the minutes left, the other tasks and how they stand.
- **The Workflows page** takes the board's place: the header's "Workflows" is lit, and it (or Esc, or a board filter)
  brings the board back. It is **one list and the task** (Buildkite, GitHub and Jira all keep the list of runs out
  of the run's own page: it is a menu or a page of its own. The emphasis follows NN/g's visual hierarchy: the most
  important thing biggest, at most two big things, bright color only for state, red for trouble):
  1. **The run, and its chain** (348 px). On top, standing still: **its name, which is the menu of your workflows**
     (a chevron beside it; the menu, under "Workflows": the ones still to run (the one that runs, with its activity
     mark; the ones due, with their time; the ones kept without a time), a rule, then **every run that ended**, the
     newest first: a lamp of how it went (green all done, amber something for you, red stuck or failed, gray stopped
     or didn't run), its name, day and time ("Yesterday, 02:00") and its tally ("1 wants your review, 1 stuck"); the
     one shown has a check; a long menu scrolls); then when and how long ("Mon 9 Nov, 02:00 · took 2 h 35 min"), how
     it went with its lamp ("Finished: 1 done, 1 stuck"), a bar of one segment per task in its state's color, the
     words for every task, its buttons (Run it again / Run now, Change, Stop, delete). Under a rule, scrolling:
     **"5 tasks"**, then the chain as the dialog draws it, with what happened where the dialog has its plan:
     "02:00, started", every task as a short row with its state on the rail: a colored icon and word (Done, Review,
     Stuck, No report, Out of time, Asked you, Cut off, Didn't run, Stopped, Running, Waiting), its number and key,
     its time, its title (two lines at most), its kind and notes; then "05:30, ended" ("stopped"), with ". Live went
     back to stock-fix." when Live moved and that workspace is still yours. A workflow that runs or waits for its
     time has no ends yet (its head says when it started or starts). The task shown is pressed in, with the blue
     bar. A run opens on the task that runs, else the first that needs you (stuck, asked, cut off,
     failed, review, no report, out of time), else the first. ↑ ↓ (or j k) go through the tasks.
  2. **The task**, across the rest: its type icon, key, Jira status and "Task 2 of 5" with Open the card and Open the
     workspace; its title, the biggest text on the page; one line of facts (kind · workspace · 02:00–02:52 · 52 of 60
     min · Live); **the one colored box**: how it stands in its state's color and tint ("Done, nothing needs you",
     "Stuck", "Running for 20 min of 45 min", "Waiting its turn: task 3 of 5, after SH-21309 …") with Claude's own
     first words in quotes, or the note on what went wrong; then what it was given, side by side when there is room:
     **"The message we sent"** (its time; "The message it will get" before it runs) and **"The Jira ticket"** (its
     description, with Open in Jira), each showing its start with "Show the whole …"; then **"What happened"**, a
     timeline in time order (the time on the left, a dot in the step's color): the message sent ("; Live followed
     it"), Claude's updates, "Enkidu asked for its report" when Claude stopped without one, the report with its
     screenshots large (a click shows one as large as the window), or how it ended when no report came. "Open the
     card" opens the task on top of the page; closed, the page is still there. The footer says "Esc back to the
     board".
- **A workflow runs once at its time; it doesn't repeat by itself.** "Run it again" (on a run) or Change with a new
  time runs it again, and every run is kept as it ended: the workflow then, each task with its state and times, in a
  table of its own (`work_flow_runs`); a report belongs to the run whose task was running on that card when it came.
  A workflow you delete leaves its runs. Workflows that ended before runs were kept become their first run, once.

### Tasks of your own, and what you set once

What it is for: a task that isn't in Jira, added by hand and set in progress; the words a kind of task starts from,
written once in Settings; and a new task made from inside the workflow dialog.

- **A task of your own is a card.** **New task**, in the board's header, opens a small dialog: a title, what it is
  about (optional; Markdown, and Claude gets it with the task), the project it is for (the one on screen unless you
  pick another) and its state. Its card carries a person in signal blue and "Your task" (signal blue is what is
  yours), then the project's name, where a ticket has its type and key. Nothing is read from anywhere and nothing is
  sent anywhere: it lives in the app's own database (`work_own_tasks`).
- **Three states: To do, In progress, Done.** What is still to do stands in a column of its own, **To do**, among
  the columns of what is to do. In progress and Done stand with Jira's tickets of that state, in the column named as
  Jira names it ("In Progress", "Done"), as a PR without a ticket does. A Jira board you picked shows them too: they
  are yours, not Jira's.
- **Its card in front of the board** is a ticket's card without Jira and GitHub: what you wrote is "The task", its
  timeline says when you made it and each state you gave it, and on the right a box **Your task** has the three
  states (a press sets it at once, and the card moves to that column), the project and when you made it. **Edit**,
  in the header, opens its dialog in front of the card: other words, another state, or **Delete**, which asks once
  more ("Keep it", "Delete it"). Hide, the filter, the keyboard and picking work as for every card.
- **It goes to Claude like a ticket.** Tell Claude types "A task of mine (not in Jira): …", what you wrote about
  it, then your words. With no Claude on it yet, Start workspace makes `task-3-try-the-small-docker-setup` on
  `feature/task-3-try-the-small-docker-setup`; that Claude is linked to the task ("linked by you"), and `enkidu report`
  there lands on its card. By name it is `--task 3` (`enkidu report done --task 3 "…"`, `enkidu note`, `enkidu update`, `enkidu link
  task 3`).
- **In a workflow** it is a task like any card (pick it with the others), told in your own words: its kind is "Your
  own words", starting from "Do this task of mine …" until you write others. **New task**, under the workflow
  dialog's tasks, makes one right there, in the workflow's project; it joins the tasks as the last one.
- **Settings → Workflows** (a page of its own, after Claude): **A new workflow** (the minutes a task gets and the
  time it starts at, 60 and 02:00 until you set them), and **What each kind of task starts from**: the words of
  Start it, Carry on, Fix what came back, Answer the review, Fix the failing checks and Investigate only, each in a
  field. Change them once; a task of that kind starts from yours, and a workflow can still change them for one
  task. While a field holds your own words, **Use Enkidu's words** puts the built-in ones back; a field you
  empty goes back to them too. ("Use these words for every …" in the workflow dialog still keeps one task's words
  as its kind's.)
- **Settings → General → Code:** the colors of code (Like an IDE, or Quiet) and the two text sizes (see "Color" and
  "Type").

### Hooks: the hand-off, the usage limit, lighter on Claude

What it is for: when a task finishes, the end of its turn is what says so, Live goes off, and the next task starts.
What Claude Code 2.1.322 offers was read from its own binary (33 hook events, four hook kinds:
command, http, prompt, agent) and the two that matter here were tried with a real Claude in a throwaway folder.

- **A task ends when Claude has reported and its turn has ended.** Before, the report alone ended it, so Live could
  move and the next task's Claude start while the first was still writing its last words. Now `enkidu report` says how
  it went and the Stop hook (the turn's end) ends it. The same hook wakes the workflow runner, which otherwise looks
  every 3 seconds, so Live moves and the next message goes out in the same second; `enkidu report` and Run now wake it
  too. A Claude that reports and keeps going is waited for, up to the task's minutes ("Its 60 minutes ran out while
  Claude was still going after its report").
- **The usage limit ends a task at once.** Claude Code runs StopFailure instead of Stop when an API error ended the
  turn, with the error's kind (`rate_limit`, `authentication_failed`, `overloaded`, `model_not_found`…) and its own
  words (`last_assistant_message`; a real Claude sent `"error":"model_not_found"` and its message). The lamp turns to
  "needs you". A workflow task is **Cut off** (red, a crossed-out bolt) with Claude Code's words as its note, and
  when every later task would hit the same error (the usage limit, the login, billing, the model) the run stops
  there and Live goes back; an API that is overloaded or failing for a while ends only that task. In a stopped run,
  a task that never got its turn says "Stopped: The run stopped before its turn" and "The message it would have got",
  not "Waiting its turn".
- **The hooks that only move the lamp run in the background** (`"async": true`: after a tool, a permission dialog, a
  question, an MCP question), so Claude no longer waits for a shell and curl (~18 ms, measured) on every tool call.
  The hooks whose answer counts (the commit guard, Stop, session start, prompt), StopFailure and SessionEnd still
  run in line. A background "working" that arrives just after the turn ended (within 1.5 s, before the next prompt)
  is ignored, so it never turns a finished Claude's lamp back on.
- **GitHub and Jira are read again at a turn's end only when the turn used them:** a push, `gh`, `twg`, `jira`,
  `acli`, or a GitHub, Jira or Atlassian MCP tool (a push still reads at once). A turn that only read and edited code
  reads nothing; the read every 5 minutes catches the rest. Claudes outside Enkidu carry the same mark through
  `enkidu event`.
- **Not done: Esc.** Claude Code runs no hook when you interrupt a tool: with a real Claude, PostToolUseFailure
  (whose input has `is_interrupt`) never fired, in line or in the background, because the interrupt cancels it. The
  terminal watch after Esc stays.

## Setup

What the app needs on a Mac, checked in one place.

- **Settings → Setup**, the last of the app's own pages (General, Claude, Workflows, Setup), with a check mark as its
  icon. It saves nothing: it looks, and says what it found.
- **Two parts.** "Enkidu needs these": Git and Claude Code. Without them no workspace starts. "Connections":
  GitHub and Jira, for the Work page. Each can be left out, and the app says so in plain words ("Not set up"), never
  as a fault.
- **A row for each thing**, on the tile the Claude folders have: a lamp, its name, at the right how it stands in one
  or two words, and under the name one line.
  - Ready (green lamp): where the program is and its version, in mono, then who it is signed in as: "`~/.local/bin/claude`,
    version 2.1.118, signed in", "`gh` 2.63.0, signed in as me-dev and me-work".
  - Something to do: what the thing is for and what to do, in one sentence, then the command to run as a chip you can
    press. A press copies it and a toast says "Copied". The lamp is red where a workspace can't start without it
    (Git, Claude Code), hollow where it is a connection you may not want.
  - While it is being looked at the row's words shimmer, as "Claude is working" does. A row never shows an old
    answer as if it were new.
- **The words at the right:** "Ready", "Not installed", "Not signed in", "Not set up" (a connection whose program is
  missing), "Can't tell" (the program ran and said something the app can't read; its own first line follows). The
  lamp carries the color. The word is gray, and red only where a workspace can't start without the thing.
- **Under Jira, when it is ready:** the projects whose tickets aren't read yet, each as a small button with the
  project's name ("Jira is read for a project once its Jira project keys are set. Set them for"). A press goes to
  that project's page, where the keys are typed.
- **Check again** is the page's one button, at the right of its first title. Every row is looked at again; nothing
  else on the page moves.
- **A build that lacks its own parts** says so in a third part, shown only then: no `enkidu` beside the program (hooks
  and `enkidu` in a terminal can't work), no `ty` (no links to definitions in Python). Each with the script that puts it
  there.
- **The first launch.** With no project yet, and Setup never shown on this Mac, Settings opens on Setup by itself,
  once. Above the first part stand a title, "Welcome to Enkidu", and two sentences: what the app is, and that
  the first project is added when this is closed. It saves nothing, so its foot has one button, Done. After that
  it is a page like the others, with Cancel and Save.
- **Nothing is installed, signed in or changed from here.** The app runs each program only to ask its version and
  who is signed in (`git --version`, `claude --version`, `claude auth status`, `gh auth status`, `twg whoami`),
  with a time limit, off the drawing thread. Account names are shown, never written to a log or a file.
- **The hooks for Claude outside Enkidu** stay on the Claude page: a row for each Claude folder, with its button.

## Scrolling in the terminal

The terminal scrolls like a native one. Its own history moves with the trackpad's pixels. A program that takes the
mouse (Claude Code in full screen, vim, less with the mouse on) gets one wheel report per line's worth of scrolling,
as Warp and iTerm send it; xterm.js alone sent one per wheel event and slowed a trackpad to 30%, so a swipe that moves
a page in Warp moved Claude a few lines (the rule is `crates/enkidu-term/src/wheel.rs`). Claude Code's own
`/scroll-speed` still tunes it.

## Big repos

See SEARCH.md: text is searched without an index (a parallel scan, streamed, about 0.2 s for a 13,500-file Python repo,
first results in milliseconds), definitions with a small index built on first use (8.7 MB for 250,000 definitions,
6 ms per keystroke; going to a definition asks it for one exact name, under 0.3 ms at 420,000 definitions, so there
is no second index), file names from git's own index. On screen: the Files list stops at 50,000 files (and says so),
a folder shows its first 300 files with "Show all", and searches keep only the best matches while they scan.

## Copy

- Name things by what you do with them: "New workspace", "Go live", "Type 3 remarks into Claude".
- Empty states say what to do next. Errors say what happened and how to fix it, without apologising.
- Status words: Idle, Working, Needs you, Done, Live.

## Where to see it

`ENKIDU_VIEW=design target/debug/enkidu-app` opens the gallery: every token and component on one page.
`cargo test -p enkidu-app --test shots` draws it offscreen to `shots/out/gallery.png`.
