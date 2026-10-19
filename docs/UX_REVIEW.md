# Interface review, October 2026

Five passes over the desktop app as it is in v1.3.0, each with one lens from
[awesome-ux-skills](https://github.com/tommyjepsen/awesome-ux-skills), on screenshots of the
running app (main window, welcome, preset gallery, video, video export and a phone's screen) and
on its code. The last pass ranks what the others found; [the plan](UX_PLAN.md) keeps that order.
Terms are the ones in [CONTEXT.md](../CONTEXT.md).

The person in mind: someone with a picture or clip, often pixel art or game footage, who wants it
to look like it is on a CRT and to save the result. The behaviour the interface is there for is
**getting from opening the app to an export they are happy with**, and coming back to that look.

## 1. General design review

**Verdict:** Needs work. The renderer, the sliders and the export window are careful work; the
path around them is not yet as clear.

**Top issues**

1. **Open and Export are only in the menus.** Nothing in the main window opens a file or saves
   the result: the empty preview says "Open an image, video or test card" without a button, and
   exporting needs the Export menu or `Ctrl+E`. A first-time user has to find both.
2. **The panel's order is not the task's order.** It opens on *Source & framing* (rotation, zoom,
   pan, transparency background), while the look itself, the preset, is not in the panel at all:
   it lives in a window behind the Presets menu. Under Undo/Redo/Reset sits a lone **General
   image** button that does not say it applies a preset, and nothing says which look is in use.
3. **What an export did disappears.** "Saved /full/path/crt.png" is the status line until the
   next preview replaces it with "Preview 1280 × 720 · 0.20s", which while editing is a moment
   later. There is no way to get to the file from the app.
4. **Broken glyphs and tooltips.** The Pause button reads "□ Pause" (no font has `Ⅱ`), `→`
   draws as a box in five places ("Configure … in Export □ Video"), and the FFmpeg setup window's
   "✘ Not found" shows a box too. The tooltips of **Pin look** and **Copy link to this look**
   have some twenty-five spaces in the middle of a sentence, from a missing line continuation.
5. **The welcome tells you who made it, not what to do.** It is the credits, rightly, and then
   "Got it — continue"; then the person is on a test card with nothing pointing at the next step.

**Recommended changes**

1. Put **Open…** and **Export PNG…** (or **Export video…**) under the source's name, and say
   which preset the look is, with a **Presets…** button beside it.
2. Keep the last saved file in the status bar, with **Show in folder**, until the next export.
3. Fall back to a font that has `→`, use glyphs the bundled fonts have elsewhere, and test that
   every character the interface writes can be drawn.
4. After the welcome, offer a first look (the plan's first-run look picker).

**Accessibility risks**

- Forty pieces of guidance are `ui.small`, 9 px Ubuntu Light: the source's size, the playback
  keys, the gallery's instructions, the export window's notes. Too small to read comfortably.
- The status light is colour only (green, amber, red); errors also show as text, which helps.
- Needs checking in use: whether Compare's divider can be moved without a pointer.

**Next step:** fix the glyphs and the save feedback, and bring Open, Export and the preset into
the panel; then try the first-run path with someone who has not seen the app.

## 2. Heuristics review

> ⚠️ Several issues to address

**H1: Visibility of system status**
- The confirmation of a save is overwritten by the next preview's timing.
- `status`, `error` and the preview's error overwrite each other, and **Dismiss** clears both
  errors (already in the plan).

**H2: Match between system and the real world**
- "Preview monitor" is a caption whose only content is a tooltip; nothing suggests it has one.
- Most CRT terms (Signal, Phase, Warm-up ticks) are right for this audience and have tooltips.

**H3: User control and freedom**
- Closing the window during an export cancels it without asking (already in the plan).

**H4: Consistency and standards**
- The playback hint says "Export frame saves this settled CRT still as PNG"; the menu item is
  **Current frame as PNG…**.
- **Project → Save project** is disabled until a project has a file, yet `Ctrl+S`, which the
  same item shows, asks where to save one.
- One row mixes three kinds of control: **Black** and **White** are toggles, **Checker** a
  checkbox, and the colour button below has no label.

**H6: Recognition rather than recall**
- Nothing outside the gallery says which preset the settings came from or how far they have
  moved from it; `model::differences` already knows.

**H8: Aesthetic and minimalist design**
- The preset gallery opens on five rows of saving, loading, help text and a storage path
  before the first preset.
- **Refresh** stays offered while Live preview is on, when it has nothing to catch up on.
- Paragraphs of always-visible fine print in *Image & output*, *Persistence* and *Color & LUT*.

**H9: Help users recognize, diagnose and recover from errors**
- Errors are `anyhow` chains as written; an FFmpeg failure does not point to **FFmpeg setup…**
  (already in the plan).

## Priority Actions
1. Bring Open, Export and the preset in use into the panel.
2. Keep the saved file, and a way to it, in view.
3. Fix the glyphs and tooltips that draw wrongly.

## 3. Cognitive load and conversion

Here a conversion is a finished export.

### Extraneous load found
- The lone **General image** button, which reads as a heading as much as a button.
- The status line's main message, most of the time, is render timing ("Preview 1280 × 720 ·
  0.20s"): numbers for a developer, in the one place a person looks for news.
- The preset gallery's save form, Load JSON, Refresh gallery, two lines of help and a full
  storage path, before any preset.
- Fine print that is always there whatever the settings, such as "Rounded glass may hide extreme
  corners even with Contain. Alpha uses the selected background. SDR output; no ICC color
  management."

### Mental model gaps
- In editors people know, Export is a button; here it is a menu of nine items mixing the PNG,
  the video, the contact sheet, three shader ports, the batch queue and FFmpeg setup (which is a
  setting, not an export).
- Presets sit in a separate window rather than above the sliders they set.
- **Reset** returns to *Super Win the Game*, which only its tooltip says.

### Offloading opportunities
1. Say which preset the look is closest to and how many settings differ, with the differences
   on hover, so nobody has to remember what they changed. *High.*
2. After an export, keep the file's name and a **Show in folder** button. *High.*
3. Offer three looks after the welcome instead of one chosen for the person. *High, more work.*
4. Step preview quality down by itself on slow graphics (in the plan). *Medium.*

### Priority recommendations
1. **Source card: Open… and Export…** — removes the hunt through the menus for the two actions
   every session needs. *High.*
2. **Preset line with Presets…** — the look in use is named where it is edited, and choosing
   another is one click from the panel. *High.*
3. **Lasting save notice with Show in folder** — the person no longer has to remember where
   they saved, or wonder whether it worked. *High.*
4. **Gallery: presets first** — the save form below *My presets*, the storage path in a
   tooltip. *Medium.*
5. **Fine print into tooltips** — keep the warnings that depend on the settings, move the
   standing notes behind a hover. *Low.*

## 4. Persuasive UX

The behaviour: a first export the person is happy with, and returning to their look.

**Reduction** — exporting takes a menu and a submenu-sized list
→ An **Export PNG…** / **Export video…** button under the source, where the eye already is.
Why it works: ability goes up for the one action the session exists for; nothing is added to
the screen that is not already a step on the path.

**Self-monitoring** — the person cannot see where their look stands
→ "Preset: Soft television · 3 changes", with the changes on hover, and the plan's **Changed
only** toggle later.
Why it works: seeing their own changes makes experimenting feel safe, and saving a preset
becomes the obvious next step once the count grows.

**Tunneling** — after the welcome there is no path, only a full panel
→ Welcome → pick one of three looks on the test card → open your own picture or keep the
card → export. Each step skippable.
Why it works: a short guided path to a first success, for people who need one; experts skip it.

**Conditioning** — a finished export is acknowledged by a status line that vanishes
→ "Saved crt.png · Show in folder" stays until the next export.
Why it works: a small, useful reward at the moment of success, which also answers "where did it
go?". No streaks, badges or prompts to share: nothing that could feel coercive.

Surveillance (social visibility) does not fit a local tool and is left out.

## 5. Feature prioritization

**Criteria.** Up: *impact on the path from opening to exporting*, judged from the reviews above.
Across: *effort*, judged from the code it touches.

| Item | Impact | Effort |
| --- | --- | --- |
| A. Glyphs that draw as boxes, and a test that catches them | High: the Pause button | Very low |
| B. Two tooltips with runs of spaces | Low | Trivial |
| C. Lasting save notice with Show in folder | High | Low |
| D. Open… and Export… in the panel | High | Low |
| E. The preset in use named in the panel, with Presets… | High | Low |
| F. Legible small print (9 px → 11 px) | Medium | Very low |
| G. "Export frame" said as the menu says it | Low | Trivial |
| H. Changed only toggle | Medium | Low |
| I. Gallery: presets first | Medium | Low |
| J. Fine print into tooltips | Low | Low |
| K. Refresh only while Live preview is off | Low | Trivial |
| L. Several dropped files go to the batch queue | Low | Low |
| M. One place for messages | High | High |
| N. First-run look picker | High | Medium |
| O. Ask before closing during an export | Medium | Medium |
| P. Adaptive preview quality | Medium | Medium |

**Matrix**

- **Do first** (high impact, low effort): A, C, D, E, F.
- **Quick wins** (lower impact, low effort): B, G, H, I, J, K, L.
- **Plan carefully** (high impact, more effort): M, N, O, P.
- **Deprioritize**: nothing found is both costly and of little use.

**Action plan.** A to G are done in this branch: the do-first items and the two trivial ones.
Then H, I, J and K together, as one pass over the panel and gallery; then N, which the preset
line makes easier, since the picker can say "Preset: General image" afterwards; then M, which
the save notice starts; then O and P. L waits until the batch queue is next worked on.

**Open questions**, since answered
- Should the first-run picker be part of the welcome window, or the step after it? The credits
  are a deliberate first screen, and the picker should not push them aside. *The step after
  it; N is done.*
- Is the default preset right for photos and screenshots, which are most first files? *Super
  Win the Game stays the default, with its NES palette off: the palette recolours only art
  drawn in the NES palette. The game's look with it is its own preset.*
