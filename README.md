# tx

A vim-style tree file explorer for the terminal. Includes a simple file preview, a text editor and a photo developer.

Runs on Linux and macOS.

## Usage

```sh
tx [PATH]
```

To have the shell `cd` to where you quit, add to your shell's rc file:

```sh
eval "$(tx --init zsh)"   # or bash
tx --init fish | source   # fish
```

`q` quits and changes directory, `<c-c>` quits without changing it.

## Keys

Press `?` for the full list. The main ones:

| Keys | Action |
|------|--------|
| `h` `j` `k` `l`, arrows | leave / down / up / enter |
| `gg` `G` `<c-d>` `<c-u>` | first / last / half page |
| `J` `K` | scroll the preview |
| `/` `n` `N`, `f` `F` `;` `,` | search, jump by first letter |
| `<cr>` | open folder, edit the file in the built-in editor, or develop a photo |
| `i` | open in your own editor or the desktop app |
| `d` `y` `x` + motion (`dd`, `yy`) | trash / copy / cut |
| `v`, `<space>` | visual range, toggle selection |
| `p` `P` | paste here / into the directory under the cursor |
| `r` `o` | rename / new file (folder if it ends in `/`) |
| `u` `<c-r>` | undo / redo file operations |
| `m{a-z}` `'{a-z}` | marks (uppercase ones persist) |
| `<c-o>` `<tab>` | jump history |
| `zh` | toggle dotfiles |
| `:` | command line: `cd`, `mkdir`, `touch`, `chmod 644`, `set hidden`, `marks`, `images`, `q` |

## Developing photos

`<cr>` on a picture (JPEG, PNG, TIFF, HEIC on macOS, or a camera raw: CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2…)
opens it beside a histogram and five panels of sliders: Basic (white balance, exposure, contrast, tones,
vibrance, saturation), Curve, HSL, Detail (clarity, texture, sharpening) and Crop. For a camera raw, temperature is in
kelvin and tint from -150 to 150, starting from the white balance the camera chose, as in Lightroom; other
pictures have shifts from -100 to 100. Edits are previewed live
and exported at full size to `<name>_edit.jpg` beside the photo, never over an existing file. As in
Lightroom, the edits are saved as you go, in a hidden XMP file beside the photo (`.DSCF0001.RAF.xmp` for
`DSCF0001.RAF`), and come back when the photo is opened again. Lightroom does not read these: the
sliders compute different things, so they are kept under tx's own namespace.

| Keys | Action |
|------|--------|
| `<tab>` `<s-tab>` | next / previous panel |
| `j` `k` | slider (Crop: move the frame) |
| `h` `l`, `H` `L` | change by a step / ten steps (Crop: `H` `J` `K` `L` resize the frame) |
| `0` | reset the slider (Crop: the crop) |
| `u` `<c-r>` | undo / redo |
| `\` | before / after |
| `[` `]` | HSL band; Crop: straighten by half a degree |
| `a` `r` | Crop: aspect ratio, quarter turn |
| `w` | export |
| `q` `<esc>` | close |

Camera raws are read with [rawler](https://github.com/dnglab/dnglab) (LGPL-2.1).

## Configuration

`~/.config/tx/config.toml` (or `$XDG_CONFIG_HOME/tx/config.toml`), every key optional:

```toml
editor = ["nvim", "code --wait"]  # first one on PATH wins; default nvim, nano
show_hidden = false
mouse = true                      # false leaves the mouse to the terminal for selecting text
images = "auto"                   # auto, kitty, sixel, iterm2, blocks, halfblocks, off
colors = "auto"                   # auto, truecolor, 256, 16, none

[keys]
"gh" = "leave"                    # command names: see COMMANDS in src/keys.rs
"zh" = ""                         # empty unbinds
```

Marks are saved in `~/.local/state/tx/marks` (or `$XDG_STATE_HOME/tx/marks`).

