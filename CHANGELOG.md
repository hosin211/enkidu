# Changelog

What changed in each version, the newest first. Each version's zip is in the [releases](../../releases).

## 0.1.0, 19 November 2026

A new window: what you read and write stands on one stage in true black, and what you choose with stands around it.

### Added

- **A rail at the left for your projects.** Work stands first, then every project, then the + that adds one; the
  command palette and Settings are at its foot. A project that is not on screen shows a red count when one of its
  Claudes needs you. (The projects were a row at the top.)
- **A project's own icon** on the rail, when its folder has one: `.idea/icon.svg` or `.idea/icon.png` first, then
  an icon or a favicon in the usual folders, then a logo. Project settings, "On the rail", can name another picture
  or ask for the letter.
- **The hidden mode** (⌘S, also in the View menu and the palette): the rail and the files go, and the stage has the
  window's width. The tabs stay, the other projects' workspaces show as small dots after them, and ⌥⌘1 and ⌥⌘2 show
  the files over the stage.
- **The switcher.** ⌃⇥ goes to the workspace you were in before. Held, it shows every workspace as a card: its
  project, its lamp and name, where Claude stands in words, the last lines of its terminal, its changes and its
  pull request. ⌃⇧⇥ goes the other way.

### Changed

- **One stage.** Claude and the changes are panes of one sheet in true black, with a line between them, an edge
  around them and 12 px corners. Each part was an island of its own on blue-gray.
- **The top row is the project on screen:** its name, which opens the project's menu, then its workspaces as tabs.
- **The files are one column at the right**, with two tabs: Changes and Files (⌥⌘1 and ⌥⌘2 show them, as they did).
- **⌃⇥ is the switcher now.** The next and the previous tab are ⇧⌘] and ⇧⌘[.
- **Blue is what is yours:** the tab of the workspace on screen, the pane your typing goes to, your remarks, a
  ticked box.
- **One scale** for every space and size: the top row 44 px, a pane's bar 40, rows and controls 28, the status
  line 28.
- **The terminal stands on true black**, and its grays have no tint.
- A tab that does not fit keeps some eight letters of its name; after that the row scrolls.
- A toast stands above the status line, and a tooltip of the rail at the button's right.
- The Work page has its header in the top row and its board on the stage.

### Fixed

- A new workspace's Changes could stay empty beside a changed file, until something else changed.
- In a narrow task card, the line that says where your message goes was cut.

## 0.0.1, 16 November 2026

The first build you can download.
