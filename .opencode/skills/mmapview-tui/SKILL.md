---
name: mmapview-tui
description: Visually inspect .mms files in mmapview-tui — drive the TUI via tui-use (start, snapshot, navigate, expand, fast-path /:N jump), verify on-screen rendering
---

## What I do

- Generate test `.mms` files with `cargo run --example generate_<name>`
- Launch `mmapview-tui` on a `.mms` file using tui-use
- Navigate the tree (expand/collapse, scroll, fast-path search)
- Verify output with snapshot
- Clean up sessions

## When to use me

When you need to **see** how a file renders in the TUI: verify schema/display
changes, test expand/collapse or fast-path behavior. For pure data extraction
use the `mmapview` skill (headless `q`/`describe`) — same checks, one command.

All commands assume `tui-use` is installed and its daemon is running.

### Build

```bash
cargo build -p mmapview-tui      # target/debug/mmapview-tui
```

### Generate an example file

```bash
cargo run --example generate_enums        # → /tmp/enums.mms
cargo run --example generate_hash_maps    # → /tmp/hash_maps.mms
```

### Start the TUI viewer

Launch in background to avoid blocking:

```bash
tui-use start mmapview-tui /tmp/enums.mms &>/dev/null &
```

Or with a custom label and terminal size:

```bash
tui-use start --label enums --cols 140 --rows 40 mmapview-tui /tmp/enums.mms &>/dev/null &
```

### Read the screen

```bash
tui-use snapshot
```

## Key input rules

`tui-use` has two ways to send input — know the difference:

| Method | Use for | Examples |
|--------|---------|---------|
| `tui-use press <key>` | Named keys only | `arrow_up`, `arrow_down`, `enter`, `escape`, `backspace`, `tab`, `home`, `end`, `page_up`, `page_down` |
| `tui-use type "<chars>"` | Printable characters | letters, digits, space, `/`, `:`, `=` |

**Gotchas:**

- `\n` inside `type` does NOT reliably send Enter — always use `tui-use press enter`
- There is no `space` named key — send space via `tui-use type " "`
- Arrow keys: `arrow_up` / `arrow_down` (the names from `tui-use keys`)
- In mmapview-tui, `/` enters search mode → characters fill the search buffer → `enter` executes

## Tree navigation

```bash
# Move selection
tui-use press arrow_down
tui-use press arrow_up
tui-use press page_down      # Scroll one page down
tui-use press page_up
tui-use press home           # Jump to first node
tui-use press end            # Jump to last node

# Expand / collapse (both Enter and Space work)
tui-use press enter
tui-use type " "

# Scroll viewport (without moving selection)
tui-use scrollup 10
tui-use scrolldown 5
```

## Fast-path search (`/:N` jump to entry)

Requires the selected node to be **inside** the target container (the
array/HashMap must appear in the path of the selected node — select a *child
element*, not the container header):

```bash
# 1. Navigate onto the container and expand it
tui-use press arrow_down
tui-use type " "
# 2. Move selection onto a child element
tui-use press arrow_down
# 3. Search mode: pattern :N for index jump, or field=value for match
tui-use type "/"
tui-use type ":99999"
tui-use press enter
```

See `src/app.rs:try_fast_path`.

### On-screen text search

```bash
tui-use type "/"
tui-use type "Active"
tui-use press enter

# Alternative — regex over the rendered screen (works in any mode)
tui-use find "Status::"
tui-use find "Color::(Red|Green)"
```

## Sessions & waiting

```bash
tui-use list                   # List sessions
tui-use use <session>          # Switch to a session
tui-use kill                   # Kill current session
tui-use wait                   # Wait for screen change (default 3 s)
tui-use wait 5000              # Custom timeout
tui-use wait --text "Color"    # Wait until screen contains pattern
```

## Full example: open enums, expand colors, jump to last entry

```bash
cargo run --example generate_enums
tui-use start --label enums --cols 140 --rows 40 mmapview-tui /tmp/enums.mms &>/dev/null &
sleep 2
tui-use snapshot
tui-use press arrow_down    # select colors
tui-use type " "            # expand colors
tui-use press arrow_down    # move into first entry
tui-use type "/"            # search mode
tui-use type ":99999"       # jump to index 99999
tui-use press enter         # execute
sleep 1
tui-use snapshot            # verify: "[99999] = Color::Yellow"
tui-use kill
```

Same check headlessly via the `mmapview` skill:
`mmapview q /tmp/enums.mms '/colors[99999]'` → `"Color::Yellow"`.
