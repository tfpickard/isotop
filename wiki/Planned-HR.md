> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#44](https://github.com/tfpickard/isotop/issues/44).

# Planned: HR

The `hr` view is specified as a planetarium sky of processes in true blackbody colours, with a key that sends every star flying to its place on a Hertzsprung-Russell diagram. In **sky mode** idle processes glow deep orange-red and busy ones white and blue-white; size and brightness follow memory on the astronomical magnitude scale; twinkle follows how erratic a process's CPU is; cgroups are constellations drawn with stick figures along socket links; kernel threads, which have no memory and therefore no luminosity, are an unresolved Milky Way band. In **diagram mode** the same stars are plotted with temperature (from CPU) increasing to the left and magnitude (from memory) on the vertical axis, and the machine's own main sequence emerges, with giants above it and white dwarfs below. Every star is classified, "K3 III" for an idle, huge process or "DA" for a small, busy one. This page is the astronomy and the colour science behind the picture, then the design as written in issue #44. Nothing here is implemented: `isotop --list-views` does not list `hr`.

![Reference mockup of the hr view in sky mode](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/hr.jpg)

![Reference mockup of the hr view in diagram mode](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/hr-diagram.jpg)

*The reference mockups (`docs/mockups/hr.jpg` and `docs/mockups/hr-diagram.jpg`), rendered in Python by `docs/mockups/code/hr.py` from synthetic data. They are the visual target, not isotop renders.*

## The system being modelled

**The diagram.** Around 1905 to 1911 Ejnar Hertzsprung noticed that among stars of the same colour, some are far more luminous than others (a distinction between "giants" and "dwarfs"), and plotted the stars of the Pleiades and Hyades clusters, which lie at a common distance, as colour against apparent magnitude. In 1913 Henry Norris Russell plotted spectral type against absolute magnitude for stars with measured parallaxes. The diagram that now carries both their names puts the surface temperature (or colour, or spectral class) on the horizontal axis, with the **hot end on the left** by astronomers' convention, and luminosity on the vertical axis. Stars do not fill it: about nine in ten lie on a diagonal band, the **main sequence**, from hot and luminous (upper left) to cool and faint (lower right); the **giants** and **supergiants** sit above it on the right, cool but luminous; the **white dwarfs** sit far below on the left, hot but faint. The structure is not a classification choice but physics: the main sequence is where stars fuse hydrogen in the core, and its position is set by mass; giants and white dwarfs are later stages.

**Why the diagram is organised by temperature.** The Stefan-Boltzmann law makes a surface of area $4\pi R^2$ at temperature $T$ radiate $L = 4\pi R^2\sigma T^4$. In log-log axes a star of radius $R$ lies on a line $\log L = 4\log T + 2\log R + \mathrm{const}$ of slope 4, so on the diagram *position off the main sequence is a statement about radius*: a cool, luminous star must be huge, and a hot, faint one tiny. The vertical axis is usually given as a bolometric magnitude, $M_\mathrm{bol} = M_\mathrm{bol,\odot} - 2.5\log_{10}(L/L_\odot)$ with $M_\mathrm{bol,\odot} = 4.74$; the magnitude scale (Pogson 1856, building on Hipparchus's six brightness classes) makes five magnitudes a factor of exactly 100 in flux, one magnitude $100^{1/5} = 2.512$, and a smaller number brighter.

**The classes.** The Harvard sequence of spectral classes, from hot to cool, **O B A F G K M** (Annie Jump Cannon's ordering of Edward Pickering's survey, recorded in the Henry Draper Catalogue), is a sequence of surface temperature; each class is split into ten subclasses 0 to 9, hottest first. The Morgan-Keenan (Yerkes) system adds **luminosity classes**: I (supergiants, with Ia brighter than Ib), II, III (giants), IV (subgiants), V (main sequence), VI (subdwarfs) and D (white dwarfs). The Sun is G2 V. A name like "K3 III" is therefore "a cool, orange giant".

**Why it fits process data.** A machine has no stellar physics, but it does have the same *organisation* in two measured quantities: a power-like spread of memory (the "luminosity" axis) and a heavy-tailed spread of CPU (the "temperature" axis), with a rough correlation between them. A process far above the machine's own trend (a huge process doing almost nothing) is a "giant", and a small, busy one is a "white dwarf". The view makes this a diagnosis: the ordinary machine has a main sequence of its own, and the classification names departures from it. It is a metaphor with real statistics underneath. It is not a claim that processes obey a mass-luminosity relation.

## History

The diagram has two origins and a naming dispute. In 1911 Ejnar Hertzsprung published, in a Potsdam observatory publication, diagrams of colour against apparent magnitude for the Pleiades and the Hyades. The stars of a cluster lie at nearly one distance, so apparent and absolute magnitude differ by a constant, and the diagram was in effect absolute magnitude against spectral type. Some sources credit his assistant Hans Otto Rosenberg with a Pleiades diagram of 1910; sources disagree on who plotted first.

Henry Norris Russell reached the same plot from the other side, independently of Hertzsprung. By 1913 enough stars had measured parallaxes to plot spectral type against absolute magnitude directly. According to the Linda Hall Library he showed the result at a joint meeting of the American Astronomical Society and the American Association for the Advancement of Science on 30 December 1913, and the talk was printed in *Nature* in three issues from 30 April 1914, the diagram first appearing in the issue of 7 May. Russell's text attributes the terms "giant" and "dwarf" to Hertzsprung, who is said not to have used them in his own papers of the time.

The horizontal axis is the Harvard spectral sequence. Pickering's survey first lettered the classes by the strength of the hydrogen lines, A strongest; Annie Jump Cannon saw that the order should follow temperature and reordered it to O B A F G K M. The Henry Draper Catalogue, published from 1918 to 1924, classified 225,300 stars on her scheme, and the International Astronomical Union adopted it in 1922. The luminosity classes were added by Morgan and Keenan in 1943. The sequence is remembered by mnemonics, the best known being "Oh, be a fine girl, kiss me".

The vertical axis is the magnitude scale, which Pogson put on a logarithmic footing in 1856: five magnitudes a factor of 100 in flux. The colours rest on Planck's law of 1900, integrated against the CIE 1931 colour matching functions. In the view the two axes are CPU and memory, so the diagram is a statistic of the machine, not an astronomical one.

## Theory

### Blackbody colour

A body in thermal equilibrium radiates the spectral radiance given by Planck's law,

$$
B_\lambda(T) = \frac{2hc^{2}}{\lambda^{5}}\,\frac{1}{e^{hc/(\lambda k_B T)} - 1},
$$

peaking at $\lambda_\mathrm{max} = 2.898\times10^{-3}\ \mathrm{m\,K}/T$ (Wien's displacement law: 502 nm for the Sun at 5772 K). A colour is what the eye (or a camera) makes of the whole spectrum: the CIE 1931 2-degree colour matching functions $\bar x(\lambda), \bar y(\lambda), \bar z(\lambda)$ reduce a spectrum to tristimulus values

$$
X = \int B_\lambda\,\bar x\,d\lambda, \qquad Y = \int B_\lambda\,\bar y\,d\lambda, \qquad Z = \int B_\lambda\,\bar z\,d\lambda,
$$

whose chromaticity $x = X/(X+Y+Z)$, $y = Y/(X+Y+Z)$ traces the **Planckian locus** as $T$ varies. The conversion to display colour is linear: $(R, G, B)_\mathrm{linear}$ from $(X, Y, Z)$ by the sRGB matrix (first row $3.2406, -1.5372, -0.4986$), negative components clipped, then the sRGB transfer function. Normalising to the brightest channel throws away the overall luminosity, which is the right thing to do here: in isotop the *brightness* of a star comes from memory, and the *hue* from temperature, and the two are kept independent.

I computed the table with the standard library, replacing the tabulated colour matching functions with the Wyman, Sloan and Shirley (2013) multi-lobe Gaussian fit, which is an approximation. Its chromaticity agrees with the values of the Planckian locus I remember to about 0.001 (for example $x, y = 0.3135, 0.3237$ at 6500 K, and $0.3449, 0.3519$ at 5000 K against $0.3451, 0.3516$; unverified against the CIE tables themselves):

| $T$ (K) | colour (sRGB bytes) | in isotop's mapping |
|---|---|---|
| 1,000 | 255, 47, 0 | below the range of any process |
| 2,400 | 255, 161, 65 | an idle process |
| 3,700 | 255, 205, 150 | start of class K |
| 5,800 | 255, 241, 235 | Sun-like |
| 6,500 | 255, 249, 254 | white |
| 10,000 | 205, 217, 255 | one busy core (A0) |
| 40,000 | 158, 184, 255 | the cap (twenty cores) |

No star is green because the Planckian locus runs from orange through white to blue and never passes through green.

A related but separate quantity is the **B-V colour index**, the difference of magnitudes in a blue and a visual band; the Sun has $B - V \approx 0.65$. A fit by Ballesteros (2012), $T = 4600\,\mathrm{K}\left[\frac{1}{0.92\,(B-V) + 1.7} + \frac{1}{0.92\,(B-V) + 0.62}\right]$, gives 5779 K for the Sun, so it is a reasonable bridge between catalogued colours and a blackbody table. The view does not use B-V.

### The mapping from process to star

The brief fixes two formulas.

* **Temperature**, with CPU in percent of one core: $T = 2400\ \mathrm{K}\,\left(1 + \mathrm{cpu}/5\right)^{0.47}$, clamped at 40,000 K. An idle process is 2,400 K (class M), 5 % CPU is 3,320 K, one full core is 10,038 K (A0), eight cores about 26,100 K, and twenty cores reach the cap, 40,000 K (O).
* **Magnitude**, from memory: $m = -2.5\log_{10}(\mathrm{memory}/1\ \mathrm{GiB})$. 1 GiB is magnitude 0, 10 MiB is $+5.03$, 100 MiB $+2.53$, 16 GiB $-3.01$. The view clamps $m$ to $[-6, +9]$, so memory 0 and everything below 0.26 MiB is $+9$, and everything above 251 GiB is $-6$.

**Spectral class and subclass.** Boundaries in kelvin: O at least 30,000; B at least 10,000; A at least 7,500; F at least 6,000; G at least 5,200; K at least 3,700; M below. The subclass digit interpolates in $\log T$ within the class (hottest is 0). The prototype's bounds for the extreme classes are 50,000 K for the top of O and 2,400 K for the bottom of M, so $T = 10{,}000$ K is A0, as in the mockup.

**Luminosity class** needs a reference. The machine has no physical main sequence, so the view *fits its own* every sample: a **Theil-Sen** regression of $m$ on $\log_{10}T$. Theil-Sen takes the median of the slopes between all pairs of points (Theil 1950, Sen 1968) and then the median of $y - \mathrm{slope}\cdot x$ as the intercept; it tolerates outliers up to a breakdown point of about 29 % ($1 - 1/\sqrt2$), which matters because a few huge idle processes would drag a least-squares fit upward. The residual $r = m - m_\mathrm{fit}$ in magnitudes (positive means fainter than the sequence) gives the class by the thresholds below. These are a convention of the view, not an astronomical standard.

| Class | residual $r$ (magnitudes) |
|---|---|
| Ia | below -6 |
| Ib | -6 to -4.5 |
| II | -4.5 to -3.25 |
| III | -3.25 to -1.75 |
| IV | -1.75 to -0.9 |
| V | -0.9 to 1.5 |
| VI | 1.5 to 2.75 |
| D (white dwarf) | 2.75 and above, if $T \ge 7{,}500$ K (otherwise VI) |

The figure below runs all of this on a *synthetic* population of 173 processes (fixed seed, memory loosely correlated with CPU): temperature against CPU with the class boundaries, the resulting diagram with the fitted line (dashed) and the class boundaries as dotted lines parallel to it, and the blackbody strip. The dots are drawn in the blackbody colour of their temperature.

![Temperature from CPU, a synthetic Hertzsprung-Russell diagram with the fitted main sequence, and the blackbody colour strip](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/hr-blackbody-diagram.svg)

*Script: `docs/figures/hr-blackbody-diagram.py`. A class label sits on the faint edge of its band (the class is the band above the line it labels). The fit shown, $m = 9.64 - 2.17\log_{10}T$, belongs to the synthetic population, not to any real machine.*

### Photometry, point spreads and flashes

How a magnitude becomes a star on screen (a sharp core inside a soft halo with sizes growing slowly with brightness) and how births and exits become nova and supernova timelines is shared with every star-drawing view: see [Math-Stellar-Photometry](Math-Stellar-Photometry). The sibling view [View-Ptolemy](View-Ptolemy) uses the same magnitude scale for its fixed stars.

### Spatial structure: constellations

On the sky the International Astronomical Union's 88 constellations tile the celestial sphere with boundaries that follow arcs of constant right ascension and declination (Delporte 1930, for the epoch 1875), which gives the stepped look the mockup imitates; the stick figures within them are *asterisms*, lines drawn by tradition between bright stars. The view takes the metaphor literally: a cgroup is a constellation, a socket link between two processes is a line of the stick figure. Nothing about where a star sits on the dome is measured: the position is a stable hash within the cgroup's region. What is measured is membership (the star is in that cgroup) and the existence and traffic of the line.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Star colour | `Process.cpu` | $T = 2400(1 + \mathrm{cpu}/5)^{0.47}$ K, cap 40,000 K, through the blackbody table (`blackbody::color`, `src/blackbody.rs`, building block B5); core colour $0.4\,\mathrm{bb} + 0.6\,\mathrm{white}$ |
| Star size and brightness | `Process.memory` (RSS; footprint on macOS) | $m = -2.5\log_{10}(\mathrm{memory}/1\ \mathrm{GiB})$, clamped $[-6, +9]$, memory 0 is $+9$; point spread by `stellar::spread`; pixel sizes scale with frame height / 720, glow radius capped at 48 px at 1080p |
| Twinkle | the shared CPU history (`History`, B1, merged) | coefficient of variation of CPU over the last 10 s of `snapshot.elapsed`, at least 3 samples; amplitude $\min(0.8,\ 0.45\,\mathrm{CV})$; sum of three sines at 0.9 to 3.5 Hz, phases from `Rng::for_identity` |
| Spectral class | $T$ | boundaries above, subclass by $\log T$ |
| Luminosity class | residual from the Theil-Sen fit of the machine | thresholds above; no class with fewer than 3 points |
| Constellation | `Process.cgroup` (name from `Process.group`) | one `layout::Discs` disc per cgroup, need $\sqrt{\mathrm{members} + 3}$ |
| Stick figure | `Snapshot.links` (sockets) and traffic (B2: `Snapshot::unix_queues`, `Snapshot::pipes`) | within a cgroup a `Line` with a gap of $4.5 + 1.6\max(0, 3.5 - m)$ px at each star, brightness $0.35 + 0.65\min(1, \log_{10}(1 + B/s)/6)$; across cgroups a faint `Beam` |
| Milky Way band | the number of `Kind::Kernel` processes | intensity $1 - e^{-\mathrm{count}/40}$; each kernel thread an unpickable `Item::Star` in the band; never on the diagram |
| Nova | a birth (`History::changes`) | `stellar` nova: +4 magnitudes in 0.3 s, e-folding 2 s, over after 6 s |
| Supernova | an exit of a process last seen above 1 GiB | flash to -6 magnitudes in 0.5 s, expanding shell, over after 8 s |
| Diagram position | the star's own CPU and memory only | $x = \log_{10}T$ from 45,000 K (left) to 2,250 K (right); $y = m$ from -6 (top) to +9 |
| Dome gradient, constellation boundaries, star position within a constellation | none | **decorative** (the dashed cgroup boundaries show membership, not measurements) |

## Design

Everything below is the specification of issue #44. None of it exists.

**Module and registry.** `src/hr.rs` with `#[derive(Default)] pub struct Hr` implementing `views::ViewModule`, and one registry row, `ViewInfo::new("hr", "HR", Backdrop::Void, build)` with `build` a `views::Build` closure, `haze` off, `links` set to `LinkRule::DrawnByView` and `flash` off (the nova and supernova replace it). The scene builds the view from its row and owns its state. There is no plugin system. The pure fit is a function in `src/hr.rs`, run once per new sample, never per frame.

**Dependencies.** Status in the current source tree:

| Item | Provides | Status |
|---|---|---|
| P1 (#17), P2 (#18), S1 (#22) | registry row, picker, parity check | merged |
| B1 (#28) | shared per-process CPU and memory history (`history.rs`) | merged |
| S4 (#25), S5 (#26), S6 (#27) | demo births and exits, `stellar.rs` helpers, source demand | merged |
| P3 (#19) | colour field with four-corner placement (`Placement::Corners` in `src/render/field.rs`), used for the dome | merged |
| B2 (#29) | Unix socket and pipe traffic for stick-figure brightness (`Snapshot::unix_queues`, `Snapshot::pipes`) | merged |
| B5 (#32) | blackbody colour table (`src/blackbody.rs`: `TABLE`, `linear`, `color`) | merged |

Every prerequisite is on `main`; the view itself is what is missing.

**Billboard dome.** A sky-plane point maps to world coordinates as `Camera::ground` of its screen offset, so the dome always faces the viewer; pan and zoom apply; rotate and tilt leave it upright. `Camera::ground` is already `pub(crate)`; `Camera::view` is private and would become `pub(crate)`. `positions`, `places` and `bounds` stay consistent with what is drawn. The dome is one opaque P3 field of about 192 x 108, placed by four world corners: a plum-to-black gradient plus the Milky Way band; rebuilt only when the number of kernel threads changes or a transition runs.

**Layout.** Each cgroup gets one `layout::Discs` disc (the issue writes `pack::Discs`; in the tree the module is `layout`). A star's offset within 0.85 of its disc radius comes from its identity hash and is relaxed against existing members once, at birth, then frozen, so a star never moves when CPU or memory change. Boundaries are the edges between cells of a fixed grid (power-distance owner), recomputed only when `Discs` places a disc or adds one, drawn as dashed lines (5 on, 4 off at 720p).

**Stars.** The S5 magnitude scale and point-spread star: a halo `Glow`, a bloom `Glow` when the flux is high, and a pickable `Sphere` core. The shared `stellar::magnitude` floors memory at 1 MiB (+7.5), so the view handles memory 0 itself to reach +9.

**Diagram mode and transition.** `m` toggles the mode (the global `d` pans, so the mockup's `d` is not used), as a `ViewKey::Local` (`m` is one of the keys a view may own), delivered to `ViewModule::key`; it is listed on the help line while hr is shown through the row's `keys`. A flag `--hr-diagram` starts a headless render in diagram mode. The transition takes 1.5 s, cubic in-out, with a per-star lag of at most 0.28 from the hash, along a quadratic Bezier with a +/-0.25 bend, driven by `stage.time`. The diagram draws axes, O B A F G K M ticks along the top and the dashed fitted line. The mode is kept in the view's own state struct.

**Fit.** Theil-Sen of $m$ on $\log_{10}T$ once per new sample, excluding kernel threads, memory-0 stars, clamped stars and pairs with $\Delta\log T \le 0.01$; above 400 stars it uses every $k$-th star by identity.

**Text.** `places` holds the constellation names and, in diagram mode, axis and region labels. The 6 most luminous stars are labelled `name K3 III`. Inspector notes show $T$, $m$, class and CV. All text goes through the existing sanitizer.

**Demo.** S4 births and exits on the registry row, with at least one exit above 1 GiB before `--time 12`; a headless warm-up of 12 one-second samples. `model::demo` is unchanged.

**Degradation.**

* Linux: full.
* macOS: available. With no cgroups, constellations are keyed by `Process.group` and the Milky Way holds `kernel_task`. If B2 has no socket traffic, stick figures are drawn at one brightness. The legend adds `constellations = app groups (no cgroups)` and `no socket traffic: lines at one brightness`.

**Tests named by the issue** include `temperature_magnitude_and_spectral_class_follow_the_brief`, `class_boundaries_fall_at_the_standard_temperatures`, `luminosity_classes_follow_the_residual_from_a_robust_main_sequence`, `theil_sen_ignores_outliers_and_tied_temperatures`, `twinkle_is_zero_for_steady_cpu_and_grows_with_variation`, `diagram_position_depends_only_on_a_stars_own_cpu_and_memory`, `the_transition_ends_exactly_on_the_diagram_after_one_and_a_half_seconds` and `only_processes_over_one_gib_go_supernova`.

## Reading the picture

* **Colour is CPU, brightness is memory, and they are independent.** The hue encodes one number, the brightness another. Because the blackbody table is normalised to its brightest channel, a hot dim star has the same brightness range as a hot bright one.
* **Class letters are CPU bins; the digit is a position within the bin.** "A0" says only that CPU is near one core. The luminosity class is relative to *this machine's own* fit, so a "giant" on a quiet machine and a "giant" on a busy one are different things; the class says "unusually large for its CPU".
* **Twinkle is a variability measure.** Its amplitude is the coefficient of variation of CPU over 10 s; a perfectly steady process does not twinkle at any CPU level, and a process with fewer than three samples in the window does not either.
* **Sky position is not data.** Which part of a constellation a star sits in tells nothing; whether it is in the constellation does.
* **Stick figures show socket links** inside a cgroup; a missing line does not prove two processes do not talk (unreadable processes, pipes, shared files).
* **The Milky Way is a count.** Kernel threads are drawn as unresolved light, their count sets the brightness, and they never appear in diagram mode.
* **Limits.** The clamps hide differences: everything above 251 GiB is one magnitude, anything under 0.26 MiB is one magnitude, and everything above twenty cores is 40,000 K.

## Try it

None of these work yet: `hr` is not registered.

```sh
# Once the view lands (issue #44).
./target/release/isotop --demo --view hr --time 12 --width 1600 --output /tmp/hr-sky.png
./target/release/isotop --demo --view hr --time 12 --hr-diagram --width 1600 --output /tmp/hr-diagram.png
./target/release/isotop --demo --view hr --renderer cpu --width 1920 --benchmark 120
```

In the running view `m` toggles sky and diagram. The figure regenerates with `python3 docs/figures/hr-blackbody-diagram.py`.

## Where in the code

* Existing: `src/stellar.rs` (`magnitude`, `spread`, `Frame::star`, `Flash`, `exit_flash`, `flash`), `src/history.rs` (`History::window`, `History::changes`), `src/layout.rs` (`Discs`), `src/simulation.rs` (`Rng::for_identity`), `src/views.rs` (`REGISTRY`, `ViewModule`, `ViewKey`), `src/model.rs` (`Process.cpu`, `Process.memory`, `Process.cgroup`, `Process.group`, `Snapshot.links`), `src/render.rs` (`Stage`, `Camera`, `label_name`), `src/palette.rs` (`kind_color`).
* Also existing, which the design uses: `src/render/field.rs` (P3), `src/blackbody.rs` (B5), `Snapshot::unix_queues` and `Snapshot::pipes` (B2).
* Planned: `src/hr.rs` and a registry row.
* Prototype, reference only: `docs/mockups/code/hr.py` (`blackbody_table`, `temperature`, `magnitude`, `spectral`, `lum_class`, `theil_sen`, the sky layout).
* Related pages: [Math-Stellar-Photometry](Math-Stellar-Photometry), [Math-Spatial-Hashing](Math-Spatial-Hashing) (a spatial hash is the tool if the one-off relaxation of stars inside a very large cgroup ever becomes costly; the issue does not specify one), [View-Ptolemy](View-Ptolemy).

## Variations and open questions

* **Isochrones and tracks.** Draw each process's last minute of $(T, m)$ as a path on the diagram, as astronomers draw evolutionary tracks, from the shared history.
* **A mass-luminosity analogue.** Measure the exponent of memory against CPU across the machine (the slope of the fit) over time and report it, as the exponent in $L \propto M^{3.5}$ is reported for stars.
* **Clusters.** Pleiades and Hyades are clusters at a common distance, which is what made Hertzsprung's diagram work. A cgroup is the equivalent: plot one cgroup alone and see whether it has a sequence of its own.
* **Colour-blind safe palette.** The only measured encoding in hue is temperature; issue #104 (colour-blind safe palette) will have to decide what to do with a hue-only variable.
* **Bolometric corrections.** Stars are drawn at their bolometric magnitude, not by band; a "V-band" variant would dim very hot and very cool stars by the bolometric correction.
* **The observer's class.** If isotop is listed among the processes it is classified like any other: about A0 at one core, M when idle. Which one a viewer sees depends on whether the frame loop is the hottest thing on the machine.
* **Dust.** Interstellar reddening could be a metaphor for memory pressure: pressure shifts colours toward red.

## References

* H. N. Russell, "Relations between the spectra and other characteristics of the stars", *Nature* 93, 252-258 (1914) (as the Linda Hall Library lists it).
* E. Hertzsprung, "Zur Strahlung der Sterne", *Zeitschrift für wissenschaftliche Photographie* 3, 429 (1905) and 5, 86 (1907) (pages from memory).
* H. N. Russell, "Relations between the spectra and other characteristics of the stars", *Popular Astronomy* 22, 275 and 331 (1914) (pages from memory).
* W. W. Morgan, P. C. Keenan, E. Kellman, *An Atlas of Stellar Spectra, with an Outline of Spectral Classification*, University of Chicago Press (1943).
* N. R. Pogson, "Magnitudes of thirty-six of the minor planets for the first day of each month of the year 1857", *Monthly Notices of the Royal Astronomical Society* 17, 12 (1856).
* C. Wyman, P.-P. Sloan, P. Shirley, "Simple analytic approximations to the CIE XYZ color matching functions", *Journal of Computer Graphics Techniques* 2(2), 1-11 (2013).
* H. Theil, "A rank-invariant method of linear and polynomial regression analysis", *Nederlandse Akademie van Wetenschappen Proceedings* 53, 386 (1950) (pages from memory).
* P. K. Sen, "Estimates of the regression coefficient based on Kendall's tau", *Journal of the American Statistical Association* 63, 1379 (1968).
* F. J. Ballesteros, "New insights into black bodies", *EPL* 97, 34008 (2012).
* E. Delporte, *Délimitation scientifique des constellations*, Cambridge University Press (1930).
* The CIE 1931 colour matching functions and the sRGB standard (IEC 61966-2-1).
