---
name: tui-use-testing
description: Run mmapview-tui on .mms files and navigate the tree using tui-use
---
## What I do

- Generate test `.mms` files with `cargo run --example generate_<name>`
- Launch `mmapview-tui` on a `.mms` file using tui-use
- Navigate the tree (expand/collapse, scroll, fast-path search)
- Verify output with snapshot
- Clean up sessions

## When to use me

Use this when you need to visually inspect an `.mms` file in `mmapview-tui` or verify that schema changes display correctly.

All `tui-use` commands below assume `tui-use` is installed and the daemon is running.

## Commands

### Generate an example file

```bash
cargo run --example generate_enums
cargo run --example generate_hash_maps
```

Writes to `/tmp/enums.mms` and `/tmp/hash_maps.mms` respectively.

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
| `tui-use type "<chars>"` | Printable characters | letters, digits, space, `/`, `:`, `=`, `-` |

**Gotchas:**

- `\n` inside `type` does NOT reliably send Enter — always use `tui-use press enter` for Enter
- There is no `space` named key — send space via `tui-use type " "`
- Arrow keys: `arrow_up` / `arrow_down` (the names from `tui-use keys`, not `Up`/`Down`)
- In mmapview-tui, `/` enters search mode → characters typed after `/` fill the search buffer → `enter` executes the search

## Tree navigation

```bash
# Move selection
tui-use press arrow_down
tui-use press arrow_up
tui-use press page_down      # Scroll one page down
tui-use press page_up
tui-use press home            # Jump to first node
tui-use press end             # Jump to last node

# Expand / collapse (both Enter and Space work)
tui-use press enter
tui-use type " "

# Scroll viewport (without moving selection)
tui-use scrollup 10
tui-use scrolldown 5
```

## Fast-path search (`/:N` jump to entry)

Fast-path requires the selected node to be **inside** the target container
(the array/HashMap must appear in the path of the selected node — this means
selecting a *child element*, not the container header).

**Correct sequence to jump to entry N:**

```bash
# 1. Navigate down to the container and expand it
tui-use press arrow_down
tui-use type " "
# 2. Move selection onto a child element
tui-use press arrow_down
# 3. Enter search mode, type pattern, execute
tui-use type "/"
tui-use type ":99999"
tui-use press enter
```

The pattern format is `:N` (empty field, numeric target) for index jump,
or `field=value` for field-value match.  See `src/app.rs:try_fast_path`.

### On-screen text search

```bash
tui-use type "/"
tui-use type "Active"
tui-use press enter

# Alternative — search rendered screen (regex, any mode)
tui-use find "Status::"
tui-use find "Color::(Red|Green)"
```

## Switch / kill sessions

```bash
tui-use list             # List sessions
tui-use use <session>    # Switch to a session
tui-use kill             # Kill current session
```

## Wait for output

```bash
tui-use wait               # Wait for screen change (default 3 s)
tui-use wait 5000          # Custom timeout
tui-use wait --text "Color"  # Wait until screen contains text
```

## Workflow

1. Generate the test file: `cargo run --example generate_enums`
2. Launch viewer in background: `tui-use start mmapview-tui /tmp/enums.mms &>/dev/null &`
3. Wait for startup: `tui-use wait 1000`
4. Take a snapshot: `tui-use snapshot`
5. Navigate / verify as needed
6. Kill: `tui-use kill`

### Full example: open enums, expand colors, jump to last entry

```bash
cargo run --example generate_enums
tui-use start --label enums --cols 140 --rows 40 mmapview-tui /tmp/enums.mms &>/dev/null &
sleep 2
tui-use snapshot
tui-use press arrow_down    # select colors
tui-use type " "            # expand colors
tui-use press arrow_down    # move into first entry
tui-use type "/"            # enter search mode
tui-use type ":99999"       # pattern: jump to index 99999
tui-use press enter         # execute
sleep 1
tui-use snapshot            # verify: "[99999] = Color::Yellow"
tui-use kill
```
