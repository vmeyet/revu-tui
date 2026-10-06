# 11 · Prose diff

Goal: read a changed Markdown file as it renders, like GitHub's rendered prose diff, with the added, removed and changed blocks marked and the changed words lit.
The raw diff stays the default; the prose view is a toggle on Markdown files.

## Opening it

`v` in the diff, on a Markdown file's row or any of its lines, shows that file as prose in the diff area; `v`, `esc` or `x` goes back to the diff, the cursor where it was.
A Markdown file is what `syntax::is_markdown` says (`.md`, `.markdown`, any case); on any other file `v` toasts `prose shows Markdown files only`.
The view keeps the review's header; its first row is the file's path and `· prose`, `muted`.
It outlives a refresh while the file is still in the MR, and closes with the MR.

## What it reads

- Both whole files, through the forge's file read at the base and head commits (`Forge::file`), cached forever per commit as `^v` and the outline do: hunks are not enough, since fences, lists and front matter need the whole file.
- A renamed file is read at its old path on base and its new one on head.
- An added file has no base side: every block reads as added. A deleted file has no head side: every block reads as removed.
- Reading runs in the background; the view shows a spinner until it ends, and a failure with `r to retry`.

## Blocks

Each side is rendered by [mrk](https://github.com/vmeyet/mrk-cli) (`mrk::markdown::render_blocks`) into blocks: a heading, a paragraph, one top-level list item, a quote, a code block, a table, a diagram.
Pairing the two sides and marking words is mrk's job (`mrk::diff::blocks`, mrk issue #15), not revu's: revu draws a list of block changes.

| Change | Drawn as |
|---|---|
| same | the block in `faded`, no bar |
| added | the block after a `▎` in `success` |
| removed | the block after a `▎` in `danger`, in `faded` |
| changed | the old block as removed, its removed words struck through in `danger`; then the new block as added, its added words underlined in `success`; on a known ground both words also get the word fill (`removed_word`, `added_word`) |

- A changed block whose old side has no word to mark (only words were added) shows its new side alone.
- One empty row between blocks; the bar takes the first column, a space the second, the block the rest.
- A run of more than 3 unchanged blocks keeps its first and last block and folds the rest into one `··· N unchanged blocks` row, `muted`; `zR` opens every run, `zM` folds them again.
- When no block changed (a rename, an edit mrk's rendering hides), the file reads as plain rendered Markdown: nothing dims and nothing folds.
- Colour is never the only cue: the bar's presence and the strike or underline carry the change on a terminal without colour.

## Rendering

- Blocks are rendered at the diff area's width less the two gutter columns, never the terminal's width.
- Rendered blocks are kept per file texts, width and theme, and rendered again only when one of them changes (a resize, `:set theme=`); scrolling renders nothing.
- mrk's colours come from revu's theme, so the view matches the rest of the screen:
  - the mrk preset of the same name when there is one (`dracula`, `catppuccin`, `catppuccin-latte`, `nord`, `tokyonight`), else `mrk-dark`, or `mrk-light` on a light ground;
  - then every RGB colour of revu's theme replaces its mrk twin: `accent`, `link`, `code`, `muted`, `faded` (mrk `subtle`), `surface`, `success`, `warn` (mrk `warning`), `danger` (mrk `caution`);
  - mrk's text colour draws as the terminal's own, as everywhere else in revu (`03-ui-ux.md` § Design rules, 7).
- Links print their target after their text; pictures are not drawn yet (see Later), so a Mermaid diagram shows as mrk's box-drawing text.

## Keys

`j` `k` scroll a row, `ctrl-d` `ctrl-u` `space` half a page, `g` `G` top and bottom, `zR` `zM` open and fold the unchanged runs, `r` reads again, `v` `esc` `x` back to the diff.

## Build

The view is a cargo feature, `prose`, on by default.

- It pulls mrk with `default-features = false` (no clap, no crossterm of its own), pinned by tag, bumped on purpose.
- On by default, because a toggle hidden behind a flag nobody sets is a feature nobody sees; the cost is about 6 MB of binary (syntect, resvg, the Mermaid engine) and mrk's minimum Rust (1.92).
- `cargo install --no-default-features` builds without it: `v` still opens the view, which says the build has no prose view, so the help and `docs/reference/keys.md` stay the same in every build.

## Later

These come in their own MRs, in this order (`06-roadmap.md`):

1. **Pairing live.** `mrk::diff::blocks` replaces revu's stand-in, which until then reads every block of the head side as same (all added or all removed for a new or deleted file).
2. **Pictures.** mrk gets the terminal's cell size, so diagrams come back as pictures drawn with `ratatui-image` at the cells mrk gives, never scaled up. A changed diagram shows its old side (red bar, `before`) above its new one (green bar, `after`), side by side when the area is wider than both.
3. **Comments.** `j` `k` move a cursor block by block. A comment on a block anchors to its `last_line`: head side, base side for a removed block. Threads show under the block holding their line. `enter` goes to the raw diff at that line, for anything that needs exact lines.
