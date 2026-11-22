# platomon 1.3

A terminal system monitor: one path-traced polyhedron per metric, rendered in monochrome ASCII on
concentric rings. Each solid's scale and spin follow its metric; the frame rate (5–150 fps) follows
total activity.

Keys: `s` toggles the stats bar, `l` the legend, `w` the wireframe overlay; `q` / `Esc` / `Ctrl-C` quit.

## Launch parameters

```
platomon [OPTIONS]
```

| Option | Syntax | Default |
|---|---|---|
| **General** | | |
| `--config` | `PATH` | `<config dir>/platomon/config.toml` if present |
| `--list` | `solids\|metrics\|aspects\|sensors\|patterns\|gradients` | prints IDs (sensors: this machine's disks/adapters; gradients: colour swatches) and exits |
| `--print-config` | flag | print effective config as TOML |
| `--snapshot` | `COLSxROWS` | render one frame to stdout, no terminal needed |
| `--snapshot-time` | seconds of animation to simulate first | 0.3 |
| `--perf-log` | `PATH`: one CSV line per second (fps, quality, render/write/wait ms, KB per frame, grid) | off |
| `--stats` | flag | stats bar visible at start |
| **Metrics & solids** | | |
| `--show` | `METRIC[,METRIC…]`, max 16 | `cpu,ram,disk,net,gpu` |
| `--solid` | `METRIC=SOLID`, repeatable | built-in mapping |
| `--ring` | `METRIC=N`, repeatable (0 = innermost ring or shell) | automatic |
| `--layout` | `rings` (on the ground) or `sphere` (concentric spherical shells) | `rings` |
| **Rendering** | | |
| `--spp` | `N` (1–64) | 2 |
| `--bounces` | `N` (0–8) | 2 |
| `--fps-min` / `--fps-max` | `FPS` | 5 / 150 |
| `--fps-invert` | flag: idle runs at `--fps-max`, full load at `--fps-min` | off |
| `--ramp` | `STR`, darkest first (any length ≥ 2) | `" .,:;-~=+*ox#%&@"` (16 levels) |
| `--char-aspect` | cell width ÷ height | 0.5 |
| **Display ratio** | | |
| `--aspect` | `fill`, preset or `W:H` | `fill` |
| `--aspect-mode` | `letterbox\|crop` | `letterbox` |
| **Camera** | | |
| `--camera` | `static\|orbit\|focus` (focus: glide toward the busiest solid) | `static` |
| `--focus-zoom` | `0..1`: 0 = only turn toward the busiest solid, 1 = close-up | 0.6 |
| `--focus-tau` | glide time constant, seconds | 1.5 |
| `--orbit-period` | seconds per revolution | 60 |
| `--orbit-dir` | `random\|cw\|ccw` (seen from above) | `random` |
| `--orbit-radius` | world units | fit to layout |
| `--orbit-height` | world units | fit to layout |
| `--orbit-bob` | vertical drift amplitude, 0 = level | 0.3 |
| `--seed` | `U64`, makes `random` repeatable | random |
| **Background** | | |
| `--bg-lum` | `0..10`: 5 = original, **0 = no background surface** | 5 |
| `--bg-pattern` | `solid\|checker\|diamond\|brick\|triangles\|hex\|radial` | `solid` |
| `--bg-contrast` | `0..1`, tile tone spread around `--bg-lum` | 0.5 |
| `--bg-scale` | tile size, world units (a full-size solid is 2 across) | 1.0 |
| `--bg-scroll` | `DX:DZ` world units/s, e.g. `0.5:0` or `-0.2:0.3` | `0:0` |
| `--bg-sway` | radius of a circular back-and-forth translation; 0 = off | 0 |
| `--bg-sway-period` | seconds per sway cycle | 10 |
| `--bg-rotate` | `off\|cw\|ccw`, about the camera's focal point (seen from above) | `off` |
| `--bg-rotate-period` | seconds per revolution | 60 |
| **Overlays** | | |
| `--legend` | flag: corner panel with a wireframe of each solid and its metric | off |
| `--legend-size` | `1..5` font size: 1–2 terminal text, 3–5 block letters (1–3 cells per font pixel); icons grow with it | 2 |
| `--legend-opacity` | `0..1` panel background: 0 = see-through, 1 = solid | 0.85 |
| `--legend-pos` | `tl\|tr\|bl\|br` (or `top-left`, …) | `tl` |
| `--wireframe` | flag: each solid's visible edges drawn in inverted luminosity | off |
| **Colour** | | |
| `--color` | `off`, a preset (`--list gradients`) or 2+ hex stops, e.g. `"#000:#f80:#fff"` | `off` |
| `--color-by` | `lum` (brightness → gradient) or `load` (each solid by its metric) | `lum` |
| `--color-depth` | `24bit` or `256` (xterm palette, for older terminals) | `24bit` |
| `--color-reverse` | flag: run the gradient backwards | off |

Any `--legend-*` option turns the legend on. `--color-by` or `--color-reverse` without `--color`
picks a default gradient: `inferno` for `lum`, `traffic` for `load`.

The orbit circles the centroid of all solids and advances with wall-clock time, so its speed doesn't
depend on fps.

### Focus camera

`--camera focus` starts from the static overview and glides toward whichever solid's metric is
busiest, looking straight at it. It moves along the line from that solid back to the overview
position: `--focus-zoom 0` stays at overview distance and only turns, `1` comes in to a close-up
(5 units from the solid's centre). Movement is exponentially smoothed with `--focus-tau` seconds.
To stop the camera darting between near-equal metrics, a new solid must lead the current one by 5
percentage points for a full second before the camera switches.

### Sphere layout

`--layout sphere` places the solids on concentric spherical shells instead of rings on the ground.
`--ring` (and automatic placement, 8 per shell) picks the shell; each shell spreads its solids
evenly (a Fibonacci lattice) and is sized so neighbours keep the same spacing as on the rings. The
whole cluster floats just above the ground plane, so the background, its patterns and contact
shadows still work; the camera frames it from further back. Solids grow and shrink about fixed
centres rather than resting on the ground.

### Colour

Colour is off by default (pure monochrome). `--color GRADIENT` maps each cell through a gradient,
using one of two sources:

- `--color-by lum`: the cell's brightness, so dark glyphs take the first stop and bright glyphs the
  last. One colour per glyph level, so colour changes only where the glyph does.
- `--color-by load`: each solid takes the colour at its metric's load (idle = first stop, full =
  last stop), shaded by brightness. The background and legend text keep the terminal's default
  colour; legend icons take their solid's colour.

| Gradient | Colours |
|---|---|
| `inferno` | black, purple, red, orange, pale yellow |
| `viridis` | purple, blue, teal, green, yellow |
| `plasma` | indigo, magenta, orange, yellow |
| `fire` | black, deep red, orange, gold, white |
| `ice` | navy, blue, cyan, white |
| `ocean` | deep blue to aqua |
| `sunset` | violet, magenta, coral, gold |
| `matrix` | phosphor green |
| `amber` | amber monochrome monitor |
| `grey` | neutral grey ramp |
| `traffic` | green, yellow, red (made for `--color-by load`) |
| `rainbow` | red through violet |

Custom gradients take two or more hex stops (`#rgb` or `#rrggbb`, `#` optional), evenly spaced,
separated by `:`, `,`, `;`, `/` or spaces. Colour escapes add to the bytes written per frame,
so very large grids on slow consoles may lose some fps; `--color-depth 256` is a little shorter.

### Luminosity levels

Each pixel's radiance is tone-mapped (Reinhard, then gamma 2.2) to a luminance from 0 to 1. The
ramp splits that range into equal buckets, one per glyph. The default ramp has 16:

| # | Glyph | Luminance | # | Glyph | Luminance |
|---:|:---:|---|---:|:---:|---|
| 0 | ` ` | 0.0000 – 0.0625 | 8 | `+` | 0.5000 – 0.5625 |
| 1 | `.` | 0.0625 – 0.1250 | 9 | `*` | 0.5625 – 0.6250 |
| 2 | `,` | 0.1250 – 0.1875 | 10 | `o` | 0.6250 – 0.6875 |
| 3 | `:` | 0.1875 – 0.2500 | 11 | `x` | 0.6875 – 0.7500 |
| 4 | `;` | 0.2500 – 0.3125 | 12 | `#` | 0.7500 – 0.8125 |
| 5 | `-` | 0.3125 – 0.3750 | 13 | `%` | 0.8125 – 0.8750 |
| 6 | `~` | 0.3750 – 0.4375 | 14 | `&` | 0.8750 – 0.9375 |
| 7 | `=` | 0.4375 – 0.5000 | 15 | `@` | 0.9375 – 1.0000 |

The original 10-level ramp is `--ramp " .:-=+*#%@"`.

### Background

The ground plane's mean brightness follows `--bg-lum` linearly: 5 is the original ground and 10 is
twice as bright. At 0 the plane is removed entirely, so the solids float in a black void with no
ground or contact shadows. Patterns alternate their tiles above and below that mean by
`--bg-contrast`, so changing the pattern doesn't change the overall brightness. `hex` uses three
tones, and `radial` is a polar checkerboard centred on the focal point. Far-away tiles smaller than
a character cell fade to the mean instead of flickering.

Motion is set in the pattern's own frame and runs on wall-clock time. `--bg-scroll` translates at a
constant velocity, `--bg-sway` adds a circular back-and-forth, and `--bg-rotate` spins the pattern
about the camera's focal point (the layout centroid the camera looks at). Rotation and the
`--camera orbit` direction use the same `cw`/`ccw` convention, so they can turn with or against
each other. With `--bg-pattern solid` (the default) motion has nothing visible to move.

| Pattern | Description |
|---|---|
| `solid` | uniform (original look) |
| `checker` | square checkerboard |
| `diamond` | checkerboard rotated 45° |
| `brick` | checkerboard, alternate rows offset half a tile |
| `triangles` | squares split on the diagonal, alternating |
| `hex` | hexagonal tiles, three tones |
| `radial` | rings × 12 sectors around the focal point |

### Legend and wireframe

The legend lists every slot in `--show` order: a small wireframe of its solid, turning with the
solid and seen from the camera's direction (visible edges as `- | / \`, hidden edges dotted),
next to the live reading (`CPU 37%`, `NIC Wi-Fi ↓1.0 ↑0.5 MB/s`). Sizes 3–5 draw the label in
3×5 block letters with a compact reading (`NIC WI-FI 1.5M`, read + write MB/s). A terminal can't
change its font for part of the screen, so the size is measured in cells. When the entries don't
fit in one column they wrap into more columns, and entries that still don't fit are left out.

`--wireframe` traces each solid's edges into the picture and inverts the luminosity under them:
dark on bright faces, bright on shadow or a black background. Mid-grey cells are pushed at least
6 glyph levels away, so edges stay visible everywhere. Edges on the far side of a solid, or behind
another solid, are hidden.

### Metrics

| Metric | Scale | Spin | Default solid |
|---|---|---|---|
| `cpu` | utilisation (minus platomon's own) | utilisation | icosahedron |
| `ram` | used ÷ total | rate of change | cube |
| `disk` | all disks, read+write | Y = read, X = write | octahedron |
| `net` | all adapters, rx+tx | Y = rx, X = tx | tetrahedron |
| `gpu` | busiest GPU's utilisation | memory-controller util, else utilisation | dodecahedron |
| `nic[:NAME]` | one adapter, rx+tx | Y = rx, X = tx | rhombic-dodecahedron |
| `disk-dev[:NAME]` | one disk/volume, read+write | Y = read, X = write | truncated-cube |

`nic` and `disk-dev` without a name follow the busiest instance and can appear several times with
different names. `platomon --list sensors` shows the names this machine has. Disks also match by
mount point, e.g. `disk-dev:C:\` or `disk-dev:/home`.

Every metric here works on Linux, macOS and Windows with any GPU vendor. Metrics that some operating
systems or vendors can't provide were left out: temperatures, CPU frequency (Windows reports a fixed
base clock), and GPU clocks/PCIe throughput (NVIDIA only). When a metric is unavailable on a
particular machine, its solid sits at minimum scale with idle spin and the stats bar shows `n/a`.

### Solids

| Family | IDs |
|---|---|
| Platonic | `tetrahedron`, `cube`, `octahedron`, `dodecahedron`, `icosahedron` |
| Archimedean | `truncated-tetrahedron`, `cuboctahedron`, `truncated-cube`, `truncated-octahedron`, `rhombicuboctahedron`, `icosidodecahedron`, `truncated-icosahedron`, `truncated-dodecahedron` |
| Catalan | `rhombic-dodecahedron`, `rhombic-triacontahedron` |

### Rings

Solids are placed on concentric rings. Without `--ring`, slots fill ring 0 (up to 8), then ring 1,
and so on. Ring numbers only set the order of the rings; empty ones are skipped, so `--ring nic=5`
on its own just makes an outer ring. A ring holding a single solid puts it at the centre. Odd rings
are offset by half a step so neighbouring rings interleave.

### Aspect presets

`vga` 4:3, `svga` 4:3, `xga` 4:3, `sxga` 5:4, `wxga` 16:10, `hd` 16:9, `wxga+` 16:10, `fhd` 16:9,
`wuxga` 16:10, `qhd` 16:9, `uwfhd` 64:27, `uwqhd` 43:18, `uhd` 16:9, `square` 1:1, or any `W:H`.
`letterbox` shrinks the picture to the ratio inside the terminal; `crop` fills the terminal with a
picture framed for the ratio and cuts off the overflow.

### Examples

Instance names (`nic:NAME`, `disk-dev:NAME`) are machine-specific; `platomon --list sensors` prints
yours. The examples use Windows names (`Ethernet`, `C:\`); on Linux use e.g. `nic:eth0` and
`disk-dev:/` or `disk-dev:nvme0n1p2`. Quote any argument that contains `\` or spaces, and quote
`--show` lists in PowerShell, which otherwise splits `cpu,ram,disk` into separate arguments.

**Getting started**

```
platomon --stats                                   # default 5 solids, stats bar on
platomon --list sensors                            # this machine's disks and adapters
platomon --list solids                             # all 15 solids with face/vertex counts
platomon --print-config > my.toml                  # start a config from the current settings
```

**Choosing metrics, solids and rings**

```
# CPU alone in the centre, everything else on a ring around it
platomon --show "cpu,ram,disk,net,gpu" --ring cpu=0 --ring ram=1 --ring disk=1 --ring net=1 --ring gpu=1

# One solid per disk on an outer ring, all truncated octahedra except C:
platomon --show "cpu,ram,gpu,disk-dev:C:\,disk-dev:D:\,disk-dev:E:\" --ring disk-dev=1 --solid disk-dev=truncated-octahedron --solid "disk-dev:C:\=rhombicuboctahedron"

# Two adapters, compared side by side
platomon --show "cpu,nic:Ethernet,nic:Wi-Fi" --solid nic:Ethernet=rhombic-dodecahedron --solid nic:Wi-Fi=rhombic-triacontahedron --stats

# Archimedean-only theme for the classic five metrics
platomon --solid cpu=truncated-icosahedron --solid ram=cuboctahedron --solid disk=truncated-octahedron --solid net=truncated-tetrahedron --solid gpu=icosidodecahedron

# All 15 solids on three rings: 1 in the centre, 6 in the middle, 8 outside.
# Named instances that don't exist on a machine simply show n/a at minimum size.
platomon --camera orbit --orbit-period 45 --stats --show "cpu,ram,disk,net,gpu,nic:Ethernet,disk-dev:C:\,nic,disk-dev,disk-dev:D:\,disk-dev:X:\,disk-dev:Z:\,nic:Wi-Fi,nic:vEthernet,nic:Bluetooth" --ring cpu=0 --ring ram=1 --ring disk=1 --ring net=1 --ring gpu=1 --ring nic:Ethernet=1 --ring "disk-dev:C:\=1" --ring nic=2 --ring disk-dev=2 --solid cpu=icosahedron --solid ram=cube --solid disk=octahedron --solid net=tetrahedron --solid gpu=dodecahedron --solid nic:Ethernet=rhombic-dodecahedron --solid "disk-dev:C:\=truncated-cube" --solid nic=rhombic-triacontahedron --solid disk-dev=cuboctahedron --solid "disk-dev:D:\=truncated-octahedron" --solid "disk-dev:X:\=rhombicuboctahedron" --solid "disk-dev:Z:\=icosidodecahedron" --solid nic:Wi-Fi=truncated-icosahedron --solid nic:vEthernet=truncated-dodecahedron --solid nic:Bluetooth=truncated-tetrahedron
```

**Camera and display ratio**

```
platomon --camera orbit --orbit-period 90 --aspect uwqhd               # slow orbit, ultrawide letterbox
platomon --camera orbit --orbit-dir ccw --orbit-bob 0 --aspect fhd     # level ccw circle, 16:9
platomon --camera orbit --seed 7                                       # random direction, same every run
platomon --aspect 21:9 --aspect-mode crop                              # fill the terminal, framed for 21:9
platomon --aspect vga --orbit-height 9 --orbit-radius 7                # 4:3, steep top-down static view
platomon --camera orbit --orbit-period 20 --orbit-radius 14 --orbit-height 3 --orbit-bob 1.5   # low, wide, bobbing fly-around
```

**Background**

```
platomon --bg-lum 1                                                    # dark floor, solids pop
platomon --bg-lum 0 --camera orbit                                     # no floor: solids floating in a void
platomon --bg-lum 8 --bg-pattern checker --bg-contrast 0.8 --bg-scale 2
platomon --bg-pattern hex --bg-scale 1.2 --bg-scroll 0.3:0.3           # three-tone hexes drifting diagonally
platomon --bg-pattern brick --bg-sway 0.6 --bg-sway-period 6           # floor swaying in a slow circle
platomon --bg-pattern triangles --bg-scale 0.6 --bg-contrast 1 --bg-scroll -0.5:0
platomon --bg-pattern radial --bg-rotate ccw --bg-rotate-period 20 --camera orbit --orbit-dir cw   # floor and camera turn against each other
platomon --bg-pattern diamond --bg-rotate cw --bg-rotate-period 30 --bg-scroll 0.4:0 --bg-sway 0.5 # rotate, scroll and sway at once
```

**Focus camera, sphere layout and colour**

```
platomon --camera focus                                                # glide toward the busiest solid
platomon --camera focus --focus-zoom 1 --focus-tau 3                   # slow, full close-ups
platomon --layout sphere --show "cpu,ram,disk,net,gpu,nic,disk-dev"    # one shell of 7
platomon --layout sphere --ring cpu=0 --ring ram=1 --ring disk=1 --ring net=1 --ring gpu=1   # CPU at the centre of a shell
platomon --color inferno                                               # brightness through inferno
platomon --color-by load                                               # each solid green → red with its load (traffic)
platomon --color viridis --color-by load --color-depth 256             # 256-colour terminals
platomon --color "#001020:#0080ff:#ffffff" --bg-lum 1                  # custom three-stop gradient
platomon --layout sphere --camera focus --color-by load --legend --wireframe   # everything at once
```

**Legend and wireframe**

```
platomon --legend --wireframe                                          # both overlays, default size/corner
platomon --legend-pos br --legend-size 1 --legend-opacity 0.5          # compact, half see-through, bottom right
platomon --legend-pos tr --legend-size 4 --legend-opacity 1            # block-letter labels on a solid panel
platomon --wireframe --bg-lum 0 --camera orbit                         # bright edges on solids floating in a void
platomon --show "cpu,ram,disk,net,gpu,nic,disk-dev" --legend-size 3 --legend-pos bl --wireframe --stats
```

**Shading and performance**

```
platomon --ramp " .:-=+*#%@"                                           # the original 10-level ramp
platomon --ramp " ░▒▓█"                                                # Unicode block shading (needs a font with these glyphs)
platomon --spp 4 --bounces 3 --fps-max 60                              # smoother, slower, capped at 60 fps
platomon --spp 1 --bounces 1 --fps-max 30                              # light on CPU
platomon --fps-min 30 --fps-max 30                                     # constant 30 fps, ignore activity
platomon --fps-invert --fps-max 60                                     # fast when idle, slows to 5 fps under load
```

**Snapshots (no terminal needed)**

```
platomon --snapshot 120x40 > frame.txt                                 # one frame of the default scene
platomon --snapshot 160x48 --snapshot-time 10 --camera orbit --orbit-period 40 --bg-pattern radial --bg-rotate ccw   # 10 s into an animation
```

**Maximum density fullscreen (Windows console)**

On a 2560×1440 display the classic Windows console's smallest bitmap font (Terminal, 4×6 px)
gives a 640 × 240 grid. Cells that size are 4:6 rather than 1:2, so pass the matching
`--char-aspect`. Set the font in the window's Properties › Font, press Alt+Enter for fullscreen, then:

```
platomon --char-aspect 0.6667 --bg-lum 1 --camera orbit --stats
```

At that resolution expect the adaptive quality to drop to `q-1`/`q-2` and tens of fps rather than 150.

## Config file

All options have TOML keys (dashes become underscores); see `config.example.toml`. Solid and ring
choices go in tables. A kind-wide key like `nic` applies to every `nic:*` slot, and an exact key
wins over it. Command-line options override the file.

```toml
show = ["cpu", "ram", "nic:Ethernet", "nic:Wi-Fi"]
camera = "orbit"
aspect = "wxga"
bg_lum = 3.0
bg_pattern = "hex"
bg_scroll = [0.3, 0.0]
bg_rotate = "ccw"

[solids]
cpu = "truncated-icosahedron"
nic = "rhombic-dodecahedron"

[rings]
nic = 1

[weights]   # activity-score weight per metric kind
cpu = 2.0
```

## Build

```
cargo build --release                       # with NVIDIA NVML support (loaded at runtime)
cargo build --release --no-default-features --features monitor # without
cargo test
cargo bench --bench render
```

## Notes

- **GPU backends**, in order: NVML (feature `nvidia`, no link-time dependency) → Linux sysfs
  `gpu_busy_percent` (AMD) → sysinfo `Gpus` (Windows PDH, macOS).
- **Feedback guard**: platomon's own CPU (per-core % ÷ cores) is subtracted from system CPU.
- **Activity score**: weighted mean over available metrics. For RAM it uses the rate of change, since
  a fill level isn't activity.
- **Adaptive quality**: when render time exceeds ~90% of the frame budget, spp drops to 1, then
  bounces to 1 (`q-1` / `q-2` in the stats bar); restored once there is headroom.
- **Polyhedra** are intersections of face planes. Vertices are recovered from the planes, and tests
  check face/vertex counts, unit circumradius and equal edge lengths for every solid.
- Every frame repaints the whole terminal area, margins included, so it doesn't depend on the terminal
  honouring clear-screen codes. Terminal size is polled each frame, for hosts that don't send resize
  events.
