# Math: Schwarzschild

The Schwarzschild metric is the exact vacuum solution of Einstein's equations outside a spherical, non-rotating mass, and nearly everything the [Horizon](View-Horizon) view draws can be derived from it in a few lines: the radii 2M, 3M and 4M and 6M, the Kepler-exact orbital frequency, the bending and capture of light, the redshift of orbiting gas, the freezing of an infalling body, the tidal stretching, even the temperature of Hawking radiation. This page derives those results, checks the important ones numerically with the Python standard library, and separates what the theory says from what the view does with it. It has no view of its own: it is the mathematics page behind [Horizon](View-Horizon), and it is independent of isotop. Throughout, $G = c = 1$ and $M$ is the mass, so lengths and times are in units of $GM/c^2$ and $GM/c^3$; for the Sun $GM/c^2 = 1.4766$ km and $GM/c^3 = 4.925\ \mu$s.

## History

The longer account lives in the repository: [docs/theory/horizon.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/horizon.md).

**Dark stars.** The idea that something could be too heavy for light to leave is older than relativity. John Michell (1783) and Pierre-Simon Laplace (1796) reasoned with Newtonian gravity and light as fast particles: enough mass inside a small enough radius gives an escape speed above $c$. The argument lapsed when light was shown to be a wave, and it was right for the wrong reasons.

**Schwarzschild.** Einstein published the field equations in November 1915. Karl Schwarzschild, an astronomer serving in the German army on the Eastern Front, sent him the exact solution for the space outside a non-rotating mass in a letter dated 22 December 1915; Einstein replied on 9 January 1916 and presented the paper to the Prussian Academy, and the solution appeared in the Academy's *Sitzungsberichte* in 1916. A second paper, on the interior of a uniform sphere, followed within weeks. Schwarzschild was invalided home with the autoimmune disease pemphigus and died at Potsdam on 11 May 1916, aged 42. His surname is German for "black shield". Nothing follows from this.

**What the radius means.** For decades the surface at $r = 2M$ was treated as a true singularity. Eddington (1924) and later Finkelstein (1958) showed that only the coordinates misbehave there and that an infalling observer crosses without incident; Kruskal (1960) gave the maximal extension used in the references. In 1939 Oppenheimer and Snyder computed the collapse of a pressureless cloud and found both pictures at once: finite proper time to the centre, and for a distant observer a surface that freezes at the horizon, which is the plunge of section "Falling in". John Wheeler popularised the phrase "black hole" in a 1967 lecture.

**Disks and pictures.** Shakura and Sunyaev (1973), Novikov and Thorne (1973) and Page and Thorne (1974) worked out what a thin relativistic disk radiates. In 1978 Jean-Pierre Luminet computed the appearance of such a disk around a Schwarzschild hole on an IBM 7040 at the Paris-Meudon Observatory and, the machine having no display, drew the result by hand in India ink; it was published in 1979. The Event Horizon Telescope imaged the shadow of M87\* in 2019 and of Sagittarius A\* in 2022. Hawking (1975) showed that the hole radiates, with the temperature of the last section, and Bekenstein (1973) had given it an entropy.

## The metric

In Schwarzschild coordinates $(t, r, \theta, \varphi)$ the line element is

$$
ds^{2} = -\left(1 - \frac{2M}{r}\right)dt^{2} + \frac{dr^{2}}{1 - 2M/r} + r^{2}\left(d\theta^{2} + \sin^{2}\theta\,d\varphi^{2}\right).
$$

$r$ is the *areal* radius (the sphere of coordinate radius $r$ has area $4\pi r^2$), $t$ is the proper time of a static observer at infinity, and the Schwarzschild radius is $r_s = 2GM/c^2$: 2.953 km for one solar mass, 29.5 km for ten, about $1.18\times10^{10}$ m for $4\times10^{6}$ solar masses. The metric is singular in these coordinates at $r = 2M$ and at $r = 0$. The first is a coordinate artefact (the curvature invariant $R_{\mu\nu\rho\sigma}R^{\mu\nu\rho\sigma} = 48M^2/r^6$ is finite there and a regular chart, such as Kruskal-Szekeres, covers it); the second is a true curvature singularity. Birkhoff's theorem says the metric is the only spherically symmetric vacuum solution, so it describes the field outside any non-rotating star as well as a black hole.

## Geodesics and the effective potential

Free particles and light follow geodesics. The metric does not depend on $t$ or $\varphi$, so two quantities are conserved along every geodesic: $E = (1 - 2M/r)\,\dot t$ (energy per unit mass) and $L = r^{2}\dot\varphi$ (angular momentum per unit mass), where the dot is $d/d\tau$ for a massive particle and $d/d\lambda$, an affine parameter, for light. Motion in a plane can be assumed by spherical symmetry. Normalising the four-velocity, $-(1 - 2M/r)\dot t^2 + \dot r^2/(1 - 2M/r) + r^2\dot\varphi^2 = -\kappa$ with $\kappa = 1$ for a massive particle and $\kappa = 0$ for light, gives

$$
\dot r^{2} = E^{2} - V(r), \qquad V(r) = \left(1 - \frac{2M}{r}\right)\left(\kappa + \frac{L^{2}}{r^{2}}\right).
$$

Writing $u = 1/r$ and eliminating the parameter with $\dot\varphi = L u^2$:

$$
\left(\frac{du}{d\varphi}\right)^{2} = \frac{E^{2} - \kappa}{L^{2}} + \frac{2M\kappa}{L^{2}}\,u - u^{2} + 2M u^{3}.
$$

Differentiating once more gives the **orbit equation**

$$
\frac{d^{2}u}{d\varphi^{2}} + u = \frac{M\kappa}{L^{2}} + 3M u^{2}.
$$

For $\kappa = 1$ the first term on the right is the Newtonian Kepler orbit and $3Mu^2$ is the whole of general relativity's correction: it produces the advance of the perihelion, about $6\pi M/(a(1 - e^2))$ per orbit for a bound orbit of semi-major axis $a$ and eccentricity $e$, which in the Solar System is 43 arcseconds per century for Mercury. For $\kappa = 0$ (light) it reads $u'' + u = 3Mu^2$, which is exactly the equation the horizon view's lens table integrates, and is the subject of the figure at the end of this page.

## Massive particles: circular orbits, the ISCO, Kepler

Circular orbits sit at extrema of $V$. With $\kappa = 1$, $V = 1 - 2/r + L^2/r^2 - 2L^2/r^3$ (units $M = 1$) and

$$
V'(r) = \frac{2}{r^{2}} - \frac{2L^{2}}{r^{3}} + \frac{6L^{2}}{r^{4}} = 0 \;\Longrightarrow\; L^{2} = \frac{r^{2}}{r - 3}, \qquad E = \frac{1 - 2/r}{\sqrt{1 - 3/r}}.
$$

Circular orbits therefore exist only for $r > 3M$ (the angular momentum diverges at $3M$). Substituting back,

$$
V''(r) = \frac{2r - 12}{r^{3}(r - 3)},
$$

which is positive, a stable minimum, if and only if $r > 6M$. The **innermost stable circular orbit** is $r = 6M$, where $L^2 = 12M^2$ and $E = \sqrt{8/9} = 0.9428$: a particle spiralling in through a sequence of circular orbits has lost $1 - E = 5.72\,\%$ of its rest energy by the time it reaches the ISCO, which is the radiative efficiency of a thin accretion disk around a Schwarzschild hole. Between $4M$ and $6M$ circular orbits exist but are unstable; at $r = 4M$ the orbit has $E = 1$, the **marginally bound orbit** (a particle at rest at infinity with $L = 4M$ would just barely be captured). The left panel of the figure below shows $V$ for several $L$: the ISCO is the inflection point of the $L^2 = 12$ curve.

The angular frequency of a circular orbit with respect to *coordinate* time follows from $\Omega = \dot\varphi/\dot t = L u^2 (1 - 2M/r)/E$, and with the expressions above simplifies to

$$
\Omega = \sqrt{\frac{M}{r^{3}}},
$$

identical to Kepler's third law, exactly and not just in the weak-field limit, in the units of the distant observer's clock. The proper time of the orbiting body runs slower: $d\tau/dt = \sqrt{1 - 3M/r}$ (that is, $u^t = 1/\sqrt{1 - 3M/r}$). The period at the ISCO is $2\pi\,6^{3/2}M = 92.3M$ in coordinate time and $65.3M$ in proper time. In isotop's units of $M = 1$ and $10M$ per wall-clock second (`orbits::M_PER_SECOND`), an ISCO orbit takes 9.2 s.

![Effective potential, Doppler beaming of a disk ring, and the plunge](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/horizon-potential-redshift-plunge.svg)

*Left: the effective potential for $L^2 = 12, 13, 16, 20$; the $L^2 = 16$ curve peaks at exactly $E = 1$ at $r = 4$. Middle: relative intensity $g^4$ around a ring of gas at inclination 80 degrees. Right: the plunge; see below. Script: `docs/figures/horizon-potential-redshift-plunge.py`.*

## Light: the photon sphere and the shadow

For $\kappa = 0$ the potential is $V = (1 - 2M/r)L^2/r^2$ and only the impact parameter $b = L/E$ matters:

$$
\left(\frac{du}{d\varphi}\right)^{2} = \frac{1}{b^{2}} - u^{2}(1 - 2Mu).
$$

A circular photon orbit needs $u' = 0$ and $u'' = 0$. The second condition, $-u + 3Mu^2 = 0$, gives $u = 1/(3M)$, so the **photon sphere** is $r = 3M$; the first then gives $1/b^2 = 1/(9M^2) - 2/(27M^2) = 1/(27M^2)$:

$$
b_c = 3\sqrt{3}\,M = 5.196\,M .
$$

The orbit is unstable: a small perturbation obeys $\delta u'' = (6Mu - 1)\,\delta u = +\delta u$, growing as $e^{\varphi}$, so a photon that lingers near $3M$ for one full turn deviates by a factor $e^{2\pi} \approx 535$. A photon arriving from infinity with $b < b_c$ is captured; with $b > b_c$ it is scattered. Seen from far away, the capture cross-section is a disk of radius $b_c$: the **shadow** of the black hole is 2.6 times wider than the horizon, and for a black hole of Event Horizon Telescope scale (M87*) this is the dark disk inside the bright ring.

**Deflection.** The total sweep of a scattered ray is $\varphi_\mathrm{tot} = 2\int_0^{u_0} du / \sqrt{1/b^2 - u^2 + 2Mu^3}$ with $u_0$ the turning point, and the deflection is $\alpha = \varphi_\mathrm{tot} - \pi$. Expanding in $M/b$ gives the weak-field result

$$
\alpha \approx \frac{4M}{b} + \frac{15\pi}{4}\left(\frac{M}{b}\right)^{2} + \frac{128}{3}\left(\frac{M}{b}\right)^{3} + \dots
$$

The leading term, $4GM/(c^2 b)$, is twice the value a Newtonian corpuscle would give, and for a ray grazing the Sun it is $4 \times 1.4766\ \mathrm{km}/6.957\times10^5\ \mathrm{km} = 1.751$ arcseconds: the deflection that was measured at the 1919 eclipse. Near the critical value the sweep diverges logarithmically (Darwin 1959; Bozza 2002):

$$
\alpha(b) \approx -\ln\!\left(\frac{b}{b_c} - 1\right) + \ln\!\left[216\,(7 - 4\sqrt{3})\right] - \pi = -\ln\!\left(\frac{b}{b_c} - 1\right) - 0.4002 .
$$

Each additional $\pi$ of deflection corresponds to $b - b_c$ shrinking by $e^{-\pi} = 0.0432$: a ray can wind around the photon sphere any number of times, so every point behind the hole has an infinite sequence of ever fainter, ever thinner images packed against the shadow's edge. The first one, the "photon ring", is at $b \approx b_c$ to within a few percent.

I integrated the orbit equation $u'' = 3u^2 - u$ with the fourth-order Runge-Kutta method (step 0.002, linear interpolation of the exit) and compared with these expansions:

| $b/M$ | numerical $\alpha$ (rad) | $4/b$ | $4/b + (15\pi/4)/b^2$ | with the cubic term |
|---|---|---|---|---|
| 6 | 1.7194 | 0.6667 | 0.9939 | 1.1914 (the series has stopped converging usefully) |
| 10 | 0.5904 | 0.4000 | 0.5178 | 0.5605 |
| 20 | 0.2361 | 0.2000 | 0.2295 | 0.2348 |
| 40 | 0.1081 | 0.1000 | 0.1074 | 0.1080 |

![Photon paths for several impact parameters and the deflection angle against b](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/schwarzschild-light-bending.svg)

*Left: rays from the left with $b = 3$ (captured), 5.25 (just outside the shadow: it winds almost once around and comes back), 6.5, 9, 14. Right: the numerical deflection, the weak-field series (dashed) and the strong-field logarithm (dashed orange). Script: `docs/figures/schwarzschild-light-bending.py`; deterministic and instantaneous to run.*

## Lensing a scene by its vertices

A renderer can lens a scene without tracing rays through every pixel. Put the observer at infinity and the hole at the origin. A point at radius $r$ whose direction from the hole makes an angle $\psi$ with the line of sight needs a photon that sweeps total angle $\psi$ to reach it (the **primary image**) or $2\pi - \psi$ (the **secondary image**, which appears on the opposite side of the shadow); in both cases the image lies at screen distance $b$ from the shadow's centre, where $b$ is the impact parameter of that photon, along the direction of the point's projection. $b(r, \psi)$ is the inverse of the sweep integral. An exact table can be built by integrating the orbit equation above for a grid of $b$ and recording the sweep at which each radius is crossed on the way in and, after periapsis, on the way out. For the primary image there is also a closed-form approximation due to Beloborodov (2002):

$$
1 - \cos\alpha = \left(1 - \cos\psi\right)\left(1 - \frac{2M}{r}\right), \qquad b = \frac{r \sin\alpha}{\sqrt{1 - 2M/r}},
$$

where $\alpha$ is the angle between the photon's direction at the emitter and the radial direction, measured by a static observer. I tested it by integrating the sweep for the $b$ it prescribes. At $r = 10M$ the sweep is reproduced to $1\times10^{-5}$ (relative) at $\alpha = 30^\circ$, $1.3\times10^{-4}$ at $60^\circ$, $7.7\times10^{-3}$ at $110^\circ$ (sweep 2.30 rad) and $0.077$ at $130^\circ$ (sweep 2.9 rad); at $r = 6M$ it is $5.9\times10^{-4}$ at $60^\circ$, $7.8\times10^{-3}$ at $90^\circ$. At $r = 20M$ the error stays below $1.1\times10^{-3}$ up to $110^\circ$. The approximation is excellent where the photon leaves the emitter outward or nearly so, degrades for photons emitted inward that must pass periapsis, and saturates at $\psi = \pi$ by construction. (These numbers are mine, from a short stdlib script; the formula's published accuracy statement is not reproduced here.)

## Redshift

A static emitter at radius $r$ is seen at infinity with frequency ratio $\nu_\infty/\nu_\mathrm{em} = \sqrt{1 - 2M/r}$: the gravitational redshift, which diverges at the horizon. For gas on a circular orbit the emitter also moves. The photon's energy at the emitter is $-p_\mu u^\mu$; with $u^\mu = u^t(1, 0, 0, \Omega)$ and the photon's conserved $E$ and $L_z$,

$$
\frac{\nu_\mathrm{em}}{\nu_\infty} = 1 + z = u^{t}\left(1 - \Omega\,\frac{L_z}{E}\right) = \frac{1}{\sqrt{1 - 3M/r}}\left(1 - \Omega\,\frac{L_z}{E}\right).
$$

The first factor combines the gravitational redshift and the transverse Doppler effect; the second is the longitudinal Doppler term, and $L_z/E$ is the photon's impact parameter component along the rotation axis of the line of sight; for an inclination $i$ it equals $-b\sin i\sin\alpha_\mathrm{img}$ (the image's screen $x$ coordinate times $\sin i$ up to sign convention), which gives Luminet's (1979) $1 + z = (1 - 3M/r)^{-1/2}\left(1 + \sqrt{M/r^3}\,b\sin i\sin\alpha\right)$. The surface brightness of a source scales as $I_\mathrm{obs} = g^{4} I_\mathrm{em}$ with $g = 1/(1+z)$ (Liouville: $I_\nu/\nu^3$ is invariant). For a ring at $r = 6M$ and $i = 80^\circ$, with no light bending, $g = 1.18$ on the approaching side and $0.50$ on the receding side, so the approaching side is $(1.18/0.50)^4 = 30$ times brighter; at $10M$ the ratio is 13 and at $20M$ it is 6. (The middle panel of the figure above.)

## Falling in: the frozen image

For a body falling radially from rest at infinity, $u^t = 1/(1 - 2M/r)$ and $dr/d\tau = -\sqrt{2M/r}$, so the coordinate velocity is $dr/dt = -(1 - 2M/r)\sqrt{2M/r}$. Close to the horizon, $dr/dt \approx -(r - 2M)/(2M)$, and the distance from the horizon decays exponentially in Schwarzschild time with an e-folding of $2M$: $r - 2M \propto e^{-t/2M}$. A distant observer, however, receives the light later than it is emitted by the travel time, which grows as the source approaches the horizon: the retarded time is $u = t - r_*$ with the tortoise coordinate $r_* = r + 2M\ln(r/2M - 1)$, and near the horizon $r_* \approx -t$, so $u \approx 2t$ and $r - 2M \propto e^{-u/4M}$. The redshift of a source falling from rest, $1 + z = 1/(1 - \sqrt{2M/r})$ for a radially outgoing photon (derived from $-p_\mu u^\mu$ with $u^t = 1/(1-2M/r)$ and $u^r = -\sqrt{2M/r}$), tends to $4M/(r - 2M)$, so it grows as $e^{u/4M}$. Two conclusions follow, both consequences of the metric and not of any model:

* the body is seen to slow, redden and dim without bound, and **never** to cross the horizon: its image freezes, exponentially fast, with an e-folding time of $4M$ of the observer's time for the redshift and for $r - 2M$;
* in proper time the body crosses without incident and reaches the singularity after a finite time: for a radial fall from rest at the horizon the proper time to $r = 0$ is exactly $\pi M$ (for one solar mass 15 microseconds; for $4\times10^6$ solar masses about 62 seconds).

I verified the exponents numerically by integrating a plunge from the ISCO ($L = 0.95\,L_\mathrm{isco}$, an inward kick of 0.1) in proper time: over $t \in [50, 58]$, $d\ln(r - 2M)/dt = -0.4999$ and $d\ln(r-2M)/du = -0.2500$ (right panel of the figure above). The factor of two matters whenever a quantity like $1 - 2M/r \propto (r - 2M)$ is used as a "brightness": it decays with $e$-folding $2M$ in Schwarzschild time and $4M$ in the observer's retarded time. The bolometric flux from a point source scales as $g^2$ (one factor for energy, one for the arrival rate), which e-folds in $2M$ of observer time.

## Tidal forces

Geodesic deviation gives the relative acceleration of two nearby freely falling particles. For a body in radial free fall, in its own frame, the tidal tensor is diagonal: stretching along the radial direction at $+2M/r^3$ and compression across it at $-M/r^3$ (the Newtonian values, and exact here). A body of length $\ell$ falling feet first feels a stretching acceleration of $2GM\ell/r^3$ between head and feet. At the horizon, $r = 2GM/c^2$:

$$
\Delta a = \frac{2GM\,\ell}{r_s^{3}} = \frac{c^{6}\,\ell}{4 G^{2}M^{2}},
$$

which falls as $M^{-2}$: for $\ell = 2$ m it is $2.1\times10^{7}\,g$ at the horizon of a $10 M_\odot$ black hole, which is fatal ("spaghettification"), and $1.3\times10^{-4}\,g$ for a $4\times10^{6} M_\odot$ black hole, which is not noticeable. A larger hole has a gentler horizon. The tides diverge as $r^{-3}$ at the singularity for every mass.

## The accretion disk flux

For a thin disk in steady state the Page-Thorne (1974) flux in the Schwarzschild case follows from conservation of mass, energy and angular momentum and the zero-torque condition at the ISCO. In units $G = c = M = 1$, with $E$, $L$, $\Omega$ the circular-orbit values above,

$$
F(r) = \frac{\dot M}{4\pi r}\,\frac{-\Omega'}{(E - \Omega L)^{2}}\int_{6}^{r}(E - \Omega L)\,L'\,dr' .
$$

Here $E - \Omega L = \sqrt{1 - 3/r}$, $-\Omega' = \tfrac32 r^{-5/2}$ and $L' = (r - 6)/\left(2(r-3)^{3/2}\right)$, so the integrand reduces to $(r' - 6)/\left(2\sqrt{r'}\,(r' - 3)\right)$ and

$$
\int_6^r \frac{r' - 6}{2\sqrt{r'}\,(r'-3)}\,dr' = \sqrt{r} - \sqrt{6} + \frac{\sqrt{3}}{2}\ln\frac{(\sqrt{r} + \sqrt{3})(\sqrt{6} - \sqrt{3})}{(\sqrt{r} - \sqrt{3})(\sqrt{6} + \sqrt{3})},
$$

(differentiate to check: the derivative of the right side is $1/(2\sqrt r) - 3/(2\sqrt r (r-3))$) giving

$$
F(r) = \frac{3\dot M}{8\pi}\,\frac{1}{(r - 3)\,r^{5/2}}\left[\sqrt{r} - \sqrt{6} + \frac{\sqrt{3}}{2}\ln\frac{(\sqrt{r} + \sqrt{3})(\sqrt{6} - \sqrt{3})}{(\sqrt{r} - \sqrt{3})(\sqrt{6} + \sqrt{3})}\right].
$$

I checked this numerically against the integral form: the ratio of the two is constant (1.5, from the factor I left out of the numerical integrand) at $r = 6.5, 7, 8, 10, 15, 20, 30, 60$, and the flux peaks at $r = 9.55M$. A version of the formula with $\sqrt3/3$ in place of $\sqrt3/2$ (which is what the Python prototype of the horizon view, `docs/mockups/code/horizon.py`, `page_thorne`, has) is not proportional to the integral (the ratio drifts from 0.29 at $r = 6.5$ to 1.38 at $r = 60$) and peaks at $8.37M$. The coefficient $\sqrt3/3$ may be a transcription of Luminet's printed equation; I have not checked the original. The brightness of the disk is proportional to $\dot M$: the observed bolometric luminosity is $\eta\,\dot M c^2$ with $\eta = 5.72\,\%$.

## Hawking temperature

A black hole radiates as a black body of temperature

$$
T_H = \frac{\hbar c^{3}}{8\pi G M k_B} = 6.17\times10^{-8}\ \mathrm{K}\;\frac{M_\odot}{M}.
$$

This is a quantum result and cannot be derived from the classical metric alone, but its form follows from a short argument. Continue to Euclidean time $t = -i\tau$ and look near the horizon, where with $\rho = 2\sqrt{2M(r - 2M)}$ the metric becomes $d\rho^2 + \rho^2 (d\tau/4M)^2 + \dots$: the plane in polar coordinates with angle $\tau/4M$. It is regular (no conical singularity) only if $\tau$ is periodic with period $8\pi M$, and a quantum field in a state regular on the horizon is then thermal at $T = 1/(8\pi M)$ in natural units (the Hartle-Hawking argument; Hawking 1975 derived it by quantising a field on the collapsing background). Restoring constants gives the formula above. The consequences: $T_H \propto 1/M$ (a black hole has negative heat capacity), a stellar-mass hole is colder than the 2.7 K microwave background and so absorbs more than it emits, and the evaporation time $5120\pi G^2M^3/(\hbar c^4)$ is $2.1\times10^{67}$ years for a solar mass. With Bekenstein's entropy $S = k_B c^3 A/(4G\hbar)$, $A = 16\pi G^2M^2/c^4$, the first law $dM\,c^2 = T\,dS$ closes the thermodynamics.

## What "memory pressure as mass" could honestly mean

The [Horizon](View-Horizon) view uses units in which $M = 1$: the mass sets the scale and nothing else, so no measured quantity is the mass of anything. The measured quantities enter elsewhere: `oom_score` ranks the orbits, memory pressure (`memory some avg10`, `snapshot.pressure[1]`) sets the *brightness* of the disk (a gain from 0.45 at no pressure to 1.35 at 100 %), and an OOM kill triggers a plunge. The honest physical content is therefore these:

* **Brightness is accretion rate.** The disk's luminosity is $\eta\dot M c^2$, so tying brightness to a pressure is tying it to a rate at which something is being consumed, which is a sound reading. It does not say how much has been consumed.
* **Rank is not mass.** The orbit radius is a rank of `oom_score`, a monotone function; the metric's radii $6M$ to $30M$ are chosen, and the physical radius of any process is not a measurement.
* **The horizon is a one-way surface, not a measurement.** What the picture claims by freezing a killed process at the horizon is an epistemic statement: the observer sees the last sample of the process, and nothing newer. That is true of isotop whichever view is shown.

## Where in the code

The [Horizon](View-Horizon) view implements part of this page; the rest is theory that the view does not use (tides, Hawking temperature, the full proper-time infall).

* [`src/horizon/lens.rs`](https://github.com/tfpickard/isotop/blob/main/src/horizon/lens.rs): `LensTable` (the photon table of section "Lensing a scene by its vertices": 360 radii by 721 sweeps, traced with RK4 at `PHOTON_STEP = 0.002`), `trace_photon`, `beloborodov_impact_parameter`, `flux` (the closed form derived above, with the coefficient $\sqrt3/2$) and `disk_flux`, `angular_velocity` and `period`, `one_plus_z`, `disk_emission`, `PlungeTrack` and `plunge_brightness`; constants `HORIZON`, `ISCO`, `SHADOW_RADIUS`, `DISK_OUTER`, `PLUNGE_FLOOR`.
* [`src/horizon/orbits.rs`](https://github.com/tfpickard/isotop/blob/main/src/horizon/orbits.rs): `rank_radius`, `band_radius`, `M_PER_SECOND`, `Orbits`; [`src/horizon.rs`](https://github.com/tfpickard/isotop/blob/main/src/horizon.rs): the view itself; [`src/horizon/disk.rs`](https://github.com/tfpickard/isotop/blob/main/src/horizon/disk.rs): the lensed disk, shadow and photon ring as triangles.
* The Python prototype, reference only, is `docs/mockups/code/horizon.py` (`LensTable`, `beloborodov_b`, `page_thorne`, `one_plus_z`, `plunge_track`); its `page_thorne` carries the wrong coefficient discussed under "The accretion disk flux", and `lens::flux` documents the correction. Figure scripts on this page: `docs/figures/schwarzschild-light-bending.py` and `docs/figures/horizon-potential-redshift-plunge.py`.

## References

* K. Schwarzschild, "Über das Gravitationsfeld eines Massenpunktes nach der Einsteinschen Theorie", *Sitzungsberichte der Königlich Preussischen Akademie der Wissenschaften* (1916), 189-196.
* M. D. Kruskal, "Maximal extension of Schwarzschild metric", *Physical Review* 119, 1743 (1960).
* C. W. Misner, K. S. Thorne, J. A. Wheeler, *Gravitation*, W. H. Freeman (1973).
* R. M. Wald, *General Relativity*, University of Chicago Press (1984).
* S. Chandrasekhar, *The Mathematical Theory of Black Holes*, Oxford University Press (1983).
* C. Darwin, "The gravity field of a particle", *Proceedings of the Royal Society A* 249, 180 (1959).
* V. Bozza, "Gravitational lensing in the strong field limit", *Physical Review D* 66, 103001 (2002).
* A. M. Beloborodov, "Gravitational bending of light near compact objects", *The Astrophysical Journal* 566, L85 (2002).
* J.-P. Luminet, "Image of a spherical black hole with thin accretion disk", *Astronomy & Astrophysics* 75, 228 (1979).
* D. N. Page, K. S. Thorne, "Disk-accretion onto a black hole. I. Time-averaged structure of accretion disk", *The Astrophysical Journal* 191, 499 (1974).
* O. James, E. von Tunzelmann, P. Franklin, K. S. Thorne, "Gravitational lensing by spinning black holes in astrophysics, and in the movie Interstellar", *Classical and Quantum Gravity* 32, 065001 (2015).
* S. W. Hawking, "Particle creation by black holes", *Communications in Mathematical Physics* 43, 199-220 (1975).
* J. D. Bekenstein, "Black holes and entropy", *Physical Review D* 7, 2333 (1973).
* F. W. Dyson, A. S. Eddington, C. Davidson, "A determination of the deflection of light by the Sun's gravitational field, from observations made at the total eclipse of May 29, 1919", *Philosophical Transactions of the Royal Society A* 220, 291 (1920).
* The Event Horizon Telescope Collaboration, "First M87 Event Horizon Telescope results. I. The shadow of the supermassive black hole", *The Astrophysical Journal Letters* 875, L1 (2019).
