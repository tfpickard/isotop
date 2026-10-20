# View Lights

A coast seen from the sea at night. Every drawn process is a lighthouse: its state selects the light's class, its threads the number of flashes in a group, and its CPU the period. Kind sets the colour, memory the nominal range, and lifetime CPU time the focal-plane height. The headlands are process houses, named by the cgroup leaf or, without a cgroup, the process group. To identify a light one has to watch it through a whole period. A still photograph of three flashes contains one flash at most.

![Lights: banded towers along a night coast, sweeping beams, flashing lanterns and glitter paths on dark water](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/lights.webp)

![The README image of the same view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/lights.webp)

## The system being modelled

A light list is an index of navigational lights. The Admiralty List of Lights and Fog Signals and the NGA *List of Lights, Radio Aids and Fog Signals* describe each light by its position, characteristic, elevation and range. A characteristic is a temporal identifier, not simply a colour. `Fl(3)W.15s 41m 23M` describes a group of three white flashes in a period of fifteen seconds, a focal plane forty-one metres high, and a nominal range of twenty-three nautical miles.

A flashing light is dark longer than it is lit. An occulting light is lit longer than it is dark; a fixed light is continuously lit. Quick and very quick lights flash rapidly at specified rates. These distinctions are carried into the view. A blocked process is mostly lit but interrupted by eclipses. A stopped process holds a fixed light; a zombie is extinguished.

A rotating lens does not switch off between flashes. Its beam remains visible in atmospheric scattering as it sweeps away from the observer. The view separates that continuous sweep from the flash observed at the lantern. Between flashes a faint warm lantern interior is decorative, not an extra flash or a change to the characteristic.

Two lights can have the same colour and different groups; a bright light in poor visibility can be seen at a shorter distance than a dimmer light in clear air. The inspector is the view's light list.

## History

The Pharos at Alexandria, built in the third century BCE, was one of the Seven Wonders of the ancient world. Its name became a word for lighthouse in several languages. Its fire was raised above the harbour; accounts of its optical arrangements are not secure, and a precise ancient range is unverified.

Augustin-Jean Fresnel presented his lighthouse lens in 1822. The stepped lens removes much of the thickness of a conventional lens while retaining its refracting surfaces. It concentrates light into directions where a mariner can use it. The first installation at Cordouan followed in 1823. A rotating lens makes a flash by sweeping a concentrated beam past the observer, rather than by turning the lamp off.

The NGA publishes its light list in publications 110 through 116; the Admiralty list is a separate series. IALA Recommendation E-110 provides rhythmic characters of lights on aids to navigation. The abbreviations in the view follow this vocabulary; assigning a characteristic to a process is isotop's analogy, not IALA's.

Etienne Allard's treatment of the propagation of light through the atmosphere gives the inverse-square law with atmospheric attenuation. Its nineteenth-century publication date is often given as 1876 (unverified). A lighthouse's stated range describes both its lamp and an assumed atmosphere and detection threshold.

## Theory

### A characteristic in time

Let $n$ be the number of flashes, $P$ the period in seconds and $\phi$ a stable identity-derived phase in cycles. The local time is

$$
\tau=(t+\phi P)\bmod P.
$$

A group flash is lit when $\tau<1.5n-1$ and $\tau\bmod1.5<0.5$. Each flash lasts half a second; consecutive flashes have one second of darkness between them. The final eclipse lasts at least two seconds, so

$$
P\geq0.5n+(n-1)+2=1.5n+1.
$$

An occulting light is the complement of this waveform. A quick light has $P=1$ s and a very quick light $P=0.5$ s; each is lit for 30 percent of its cycle. A fixed light has intensity fraction one, an extinguished light zero. `waveform` computes this directly: no frame counter or numerical integrator participates.

The thread code is

$$
n=\operatorname{clamp}(\operatorname{round}(\log_2\max(N,1))+1,1,6).
$$

For mean CPU $c$ in percent of one core over the available last thirty seconds, the desired period is $30/(1+c/15)$ s. `quantize_period` chooses the smallest member of $\{2.5,3,4,5,6,7.5,10,12,15,20,30\}$ no shorter than that period or the group's minimum.

For a rotating group lens, beam $k$ has azimuth relative to the viewer

$$
\theta_k=2\pi\left(\frac{t}{P}+\phi-\frac{0.25+1.5k}{P}\right),\qquad k=0,\ldots,n-1.
$$

Thus each beam faces the viewer at its flash's midpoint. The beams are not evenly spaced: the larger gap after the last beam preserves the long group eclipse. Quick and very quick lenses have one beam, facing the viewer at $0.15P$ into their cycle. The sweep is continuous and uses absolute time. Fixed and occulting lanterns do not draw rotating beams.

### Allard's law

The memory-to-intensity transform spans five decades in candela from one MiB to ten GiB:

$$
I=10^{1+5\operatorname{clamp}(\log_{10}(\max(m,2^{20})/2^{20})/\log_{10}(10240),0,1)}\;\mathrm{cd}.
$$

For distance $d$ in metres and atmospheric transmissivity $T$ per nautical mile,

$$
E=\frac{I\,T^{d/1852}}{d^2},\qquad E_0=2\times10^{-7}\;\mathrm{lux}.
$$

`allard` bisects $d$ between zero and $\sqrt{I/E_0}$ for 64 iterations, then divides by 1852 to report nautical miles. Nominal range uses $T=0.74$. For a hand check, a ten-mile light needs $I=E_0(18520)^2/0.74^{10}\approx1393.140004$ cd. The test solves that intensity back to ten miles.

I/O stall pressure $p$ gives meteorological visibility and atmospheric transmission

$$
V=\frac{20}{1+p/5}\;\mathrm{M},\qquad T=0.05^{1/V}.
$$

The five-percent term is the contrast threshold in the meteorological visibility definition. Pressure is not an atmospheric measurement: it is the measured input to the view's analogy. Luminous range is not geographic range; it does not account for the curvature of the Earth or the observer's height.

## Mapping

| Visual element | Measured source | Transform |
|---|---|---|
| Lighthouse | Process identity | The 64 largest drawn processes by memory, ties by identity; every sampled process keeps a seat even when not drawn |
| Headland | Cgroup leaf, else process group | One named headland per house, at a stable slot on a gentle arc; seats in four columns, 2.3 units across and 2.8 behind |
| Class | State and current CPU | Z extinguished; T or traced t fixed; D group occulting; otherwise CPU at least 200% very quick, R or CPU at least 80% quick; others group flashing |
| Group count | Threads | clamp(round(log2(max(threads, 1))) + 1, 1, 6) |
| Group period | Shared CPU history, else current CPU | Mean over the available last 30 s; desired 30 / (1 + CPU / 15), rounded upward to the published periods and the group minimum |
| Light colour | Kind | System W white, session R red, container G green, kernel Y yellow |
| Tower bands | Kind | System white; session red/white; container and kernel black/white |
| Candela and nominal range | Memory, RSS on Linux and physical footprint on macOS | Log-linear 10 cd at 1 MiB to 1e6 cd at 10 GiB, clamped; Allard with T = 0.74 |
| Lantern bloom | Luminous range and waveform | Outer radius min(22 + 3 range, 115) pixels at 900 px height; outer strength 0.55 times waveform, inner bloom and star core only during flashes |
| Rotating beams | Characteristic, luminous range and I/O pressure | n beams for Fl(n), one for Q/VQ; angle from absolute phase, length 8 + 2.8 range world units; peak scattering 0.12 + 0.24 p / (p + 15) |
| Glitter path | Waveform | Decorative broken water streaks, colour mixed from local water illumination towards light with weight 0.025 + 0.45 times waveform, fading towards the foreground |
| Fog and luminous range | I/O pressure | V = 20 / (1 + p / 5) M, T = 0.05^(1 / V), re-solved by Allard; Gaussian coastal fog peak 0.08 + 0.3 p / (p + 15) |
| Focal-plane height | Lifetime CPU seconds | min(8 + 12 log10(1 + CPU time), 80) m; tower height is one fifth of this in world units |
| Establishment and extinction | Consecutive sampled identities | A 1.5 s brightening on establishment; a 1.5 s fade at the last seat on extinction; newest three Notices to Mariners |
| Inspector | Characteristic, height, nominal and luminous range | Full notation and words; units m, M and seconds |
| Tour | Nominal range | Brightest range first, ties by identity |

**Decorative:** the headlands' rugged silhouette and arc, tower taper and roofs, the faint warm lantern interior between flashes, sea colour and swell, glitter shape and shimmer, beam spread and slight upward tilt, the deep-blue sky gradient, stars, the clear-weather baseline fog, and the phase hash. The measured waveform alone drives strong lantern bloom. Beam azimuth faces the viewer at each flash's midpoint. Fog increases scattering while shortening the measured luminous range. The coast is not a map of any real shore.

## How isotop realises it

`Lights` is one module behind `ViewModule`. `seat` calls `layout::Seats` with every sampled identity, not the admitted slice. Surviving houses keep their headland slots. The camera envelope uses the sampled population, so focus and limits neither relocate towers nor reframe the coast. Its envelope reserves foreground sea rather than empty sky; the default camera retains the registry's low pitch.

`draw` budgets at most `CAP = 64` lights, including fading exits, admits the living processes with `model::top_n`, reads the CPU window through `stage.history`, and builds the frame's `Lamp` list. `night` composes sky, water, a continuous rugged land silhouette, coastal fog and rotating beams into a colour field with one node per four frame pixels, each dimension capped at 512 nodes. Water swell and the decorative ridge and shore follow world coordinates under the camera. `sweep` paints Gaussian wedges whose spread grows from 2 to 20 pixels at 900 px height and whose strength fades along the beam. Composing the colours before the shared field primitive samples them prevents accumulated backend blending differences from overlapping beams.

The field is a screen-space panel behind tower geometry. A faint ridge rim and triangle-based shoreline strokes separate dark land from sky and water. Towers are triangles; their walls and lantern carry their process index. Lantern bloom and stars use rounded `Item::Puff` splats; glitter and star cores use triangle-based hairlines. `shine` samples the field's local illumination before mixing glitter colour, so dim reflections do not cut dark stripes into illuminated water. No rasterizer behaviour is added.

`record` runs on every new sample, whichever view is shown. It compares identities with the previous sample, keeps the last seat of an exit, and records establishments and extinctions. Initial processes are not announcements of new lights. Notices and exit records are retained for 180 seconds, with at most 1024 of each; only the newest three notices as of the shown snapshot are displayed. At most eight fading exit glows reserve places within both the light cap and `stage.limit`; already unlit zombies have no exit glow. A focused subtree suppresses exit decorations. Empty houses are forgotten and their slots can be reused without moving surviving houses. The demo requests churn and exact CPU time from its registry row.

The legend begins with drawn and collapsed counts and visibility, then missing-source notes and static mappings. `stage.titles` names the headlands, `stage.tour` orders the lights, and `stage.popup` carries the light list. Process and house text follows the ordinary terminal sanitisation path.

## Reading the picture

Watch one lantern rather than the whole coast. Count its flashes, wait through the long eclipse, and compare the next group. A short sequence without the final eclipse does not establish its group count. Quick flashes indicate running or heavy CPU; an occulting light indicates an uninterruptible wait; a fixed light is a stopped process, not an idle one.

Colour tells kind. Tower height is accumulated CPU, not present load. Range grows logarithmically with memory-derived intensity and falls with I/O pressure. A light whose strong bloom is absent may be between flashes, with its lens beam sweeping elsewhere. A zombie has no waveform or rotating beam at all. The inspector separates these assignments from impressions of brightness; a headland's apparent position has no relation to geography or network addresses.

**What cannot be measured.** Missing I/O pressure means clear weather and nominal range; the legend says so through `status::missing_note`. macOS reports no I/O PSI and no cgroups, so it uses process groups and clear weather; it never reports state D. Both platforms can miss processes which live entirely between samples. The view announces sampled establishments and extinctions, not every kernel life event. Memory-derived candela and CPU-derived elevation are analogies, not properties of an executable.

## Try it

```sh
./target/release/isotop --demo --view lights --time 12 --width 1600 --renderer cpu --output /tmp/lights-12.png
./target/release/isotop --demo --view lights --time 20 --width 1600 --renderer cpu --output /tmp/lights-20.png
./target/release/isotop --view lights
```

Use `v` to choose Lights, `l` to hide labels, and `g` to visit the brightest lights. Click a tower or lantern for its characteristic. Leave the view running through a complete period; the longest is thirty seconds. Lights has no local keys.

## Where in the code

* `src/lights.rs`: `Lights`, `Lamp`, `Class`, `Characteristic`, `characteristic`, `quantize_period`, `waveform`, `candela`, `allard`, `visibility`, `luminous_range`, `fog`, `house`, `headland`, `night`, `tower`, `beam_angle`, `sweep`, `shine`.
* `src/views.rs`: the final `lights` registry row; `src/main.rs`: the module declaration.
* `src/layout.rs`: `Seats`; `src/history.rs`: `History::window`; `src/model.rs`: `Process`, `top_n` and the opt-in demo.
* `src/render/field.rs`: `Field`, `Placement`, `Relief` and `Frame::field`; `src/render.rs`: picks, rounded splats, strokes and stage outputs.
* Tests cover the ten-mile Allard reference, state precedence, group counts, period quantisation, beam alignment at each flash midpoint, fog degradation, stable seats, sample-diff notices and deterministic frames.

## Variations and open questions

1. Distinguishing two lights with the same characteristic requires a position in a real light list. The coast gives positions but no real coordinates.
2. Group membership can change when threads change. A light may acquire a new characteristic halfway through observation, unlike a maintained navigational aid.
3. Geographic range could be shown separately using focal-plane and observer heights; it would be another derived analogy, not an additional measurement.
4. Actual colours have chromaticity bounds. The rendered RGB colours are recognisable IALA names, not calibrated navigational sources.
5. A temporal legend could draw a single period of the selected light without replacing the need to watch the lantern.

## References

* IALA Recommendation E-110, *Rhythmic Characters of Lights on Aids to Navigation*.
* UK Hydrographic Office, *Admiralty List of Lights and Fog Signals*.
* National Geospatial-Intelligence Agency, *List of Lights, Radio Aids and Fog Signals*, publications 110-116.
* IALA, *International Dictionary of Marine Aids to Navigation*: nominal range, luminous range and Allard's law.
* Augustin-Jean Fresnel, *Memoire sur un nouveau systeme d'eclairage des phares* (1822).
* [Theory History](Theory-History), [Theory Stable Layout](Theory-Stable-Layout), [Theory Determinism](Theory-Determinism), [Theory Picking](Theory-Picking).
