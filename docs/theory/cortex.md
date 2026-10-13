# Cortex: processes as spiking neurons

**View:** `cortex` · **Code:** `src/neurons.rs` (the simulation, no drawing), `src/cortex.rs` (layout and drawing) · **Prototype:** `docs/mockups/code/cortex.py`

## What you are looking at

| On screen | Measured quantity |
|---|---|
| A neuron per process | the process |
| Which layer, and the firing pattern | the process kind (kernel, system, session, container) |
| How often it fires | CPU use, through a calibrated current |
| Dendrites | threads, one to twelve of them: the count is the process's thread count clamped to 1..12 (`MAX_PRIMARIES`), so a process with 500 threads draws 12 and a count of zero draws one |
| An axon between two neurons | a socket or pipe link between the processes |
| Synaptic weight of an axon | traffic on the link in bytes per second; the socket count when traffic is unmeasured |
| The raster along the bottom | the last ten seconds of spikes |
| The synchrony in the legend | Golomb's measure over neurons that talk |

The neuron model is real and standard. The *mapping* from CPU to current is an engineering choice, covered below.

## Background theory

### Izhikevich neurons

A biological neuron integrates input current into a membrane potential; when the potential crosses a threshold it spikes and resets. Hodgkin-Huxley models do this faithfully with four coupled equations and are too expensive for thousands of cells. Izhikevich (2003) found a two-variable model that reproduces a large zoo of firing patterns for a few operations a step:

$$v' = 0.04\,v^2 + 5v + 140 - u + I \qquad u' = a\,(b\,v - u)$$

with a reset rule: if $v \ge 30\ \mathrm{mV}$, then $v \leftarrow c$ and $u \leftarrow u + d$.

- $v$ is the membrane potential in mV, $u$ a recovery variable (a lumped stand-in for potassium activation and sodium inactivation), $I$ the input current.
- $a$ is the recovery rate, $b$ its sensitivity to $v$, $c$ the post-spike reset potential, $d$ the post-spike jump in recovery.
- The quadratic is a fit to the shape of a real spike upstroke, not a derived quantity.

The model is a normal form near a saddle-node bifurcation on an invariant circle. Below the **rheobase**, the smallest current that makes the neuron fire repeatedly, it sits at rest. Above it, it fires, and the rate rises smoothly from zero. That is Type I excitability, and it is why a lightly loaded process fires rarely but not never.

### Firing patterns

Four of the paper's patterns are used:

| Code | Pattern | $a$ | $b$ | $c$ | $d$ | Kind |
|---|---|---|---|---|---|---|
| FS | fast spiking | 0.1 | 0.2 | -65 | 2 | kernel |
| RS | regular spiking | 0.02 | 0.2 | -65 | 8 | system |
| CH | chattering | 0.02 | 0.2 | -50 | 2 | session |
| IB | intrinsically bursting | 0.02 | 0.2 | -55 | 4 | container |

Fast-spiking cells are inhibitory interneurons that follow strong drive with no adaptation. Regular-spiking cells are the cortex's main excitatory cell: a large $d$ makes them adapt, so inter-spike intervals lengthen. A high reset potential $c$ keeps chattering and bursting cells close to threshold after a spike, so they fire in bursts. The assignment of kind to pattern is a design choice (kernel threads are fast and relentless, services are steady, sessions are chatty).

### Synapses and delays

When a neuron fires, a spike leaves along each axon and arrives after a conduction delay (at least one tick). On arrival the target takes a pulse of synaptic current $w$ that decays with a time constant of 5 ms:

$$I_{syn}(t + \Delta t) = I_{syn}(t)\,e^{-\Delta t/\tau}, \qquad \tau = 5\ \mathrm{ms}$$

Spikes in flight live in one priority queue ordered by arrival tick. Because every step is a whole number of 0.5 ms ticks, two runs that take the same steps give the same spikes, whatever the frame rate that asked for them.

### Synchrony

Golomb and Rinzel's synchrony measure asks how much of each neuron's variance over time is shared with the population mean:

$$\chi = \sqrt{\frac{\mathrm{Var}_t\big(\langle V \rangle\big)}{\big\langle \mathrm{Var}_t(V_i) \big\rangle_i}}$$

$\chi = 0$ for neurons that never agree and $\chi = 1$ for neurons that always do. `Network::synchrony` computes it from exponential running averages of $V$ and $V^2$ with a time constant of 2 s, so it needs no stored history. It reports "unknown" for fewer than two neurons or for neurons that hardly vary, instead of 0.

## How isotop implements it

**Integration.** `advance` takes one explicit Euler step of 0.5 ms, clamps $v$ at 30 mV, and applies the reset when it sees $v \ge 30$ at the start of the next step.

**CPU to current.** The same CPU should not mean the same current for an FS and an IB cell, because they have different rheobases. `neurons::calibration` therefore *measures* two currents per firing type at startup, without noise: the one that gives 1 spike/s (`rheobase`) and the one that gives 40 spikes/s (`forty_hertz`), by scan then bisection to 0.001. `current` then interpolates piecewise-linearly between landmarks:

| CPU | current |
|---|---|
| 0 % | 0 |
| 0.5 % | 0.62 × rheobase (well below firing) |
| 5 % | 0.98 × rheobase (just short of firing) |
| one core | the 40 Hz current |
| three cores or more | twice as far above the 40 Hz current as 40 Hz is above rheobase |

An active neuron also gets a little Gaussian current noise per step (standard deviation 1.2), seeded from process identity, which keeps neurons from firing in lockstep when their inputs are equal.

**Wiring.** Axons come from the sample's socket links and pipes. A socket link's `weight_of_rate` gives its synapse $w = \min(2,\ \tfrac14 \ln(1 + \text{bytes/s} \,/\, 1000))$, and when traffic is not measured, `weight_of_sockets` gives the same log of the socket count; the legend says which is in use. A pipe link is different: it always gets the fixed `PIPE_WEIGHT` of 1.0, so a pipe axon says that two processes are connected and nothing about how much flows through it. Both directions are offered when a link is two-way.

**Layout.** A process's seat in its layer is fixed for its life (`layout::Seats`). A layer that outgrows the slice widens it by 1.25× and the slice never narrows again, so a seat never changes with the population. The one exception is structural growth: when an arriving process makes a layer exceed its capacity the slice widens, and every neuron's position is recomputed against the new width (seats stay the same, positions scale). Exits never shrink it.

## Faithful, stylized, invented

- **Faithful:** the Izhikevich equations and parameters, Euler stepping at 0.5 ms, the reset rule, conduction delays, exponential synaptic decay, Golomb's $\chi$.
- **Stylized:** one synapse type (excitatory, current-based), logarithmic weights, kinds as cell classes, the specific CPU landmarks above.
- **Invented:** that a process is a neuron at all. No claim is made that CPU load is a current, or that a pipe is an axon. The picture shows what *would* happen if it were, and synchrony then measures real structure in the process graph: two busy processes that talk fire together.

## Reading the picture

- A kernel thread under load fires fast and regularly; a container fires in bursts; a quiet system service is silent or fires on noise.
- Pulses travelling along an axon are spikes in flight; the delay is why a downstream neuron lags.
- Synchrony near 0 means the talkers are independent. A rise says that busy processes joined by traffic are firing together.
- Vertical stripes in the raster are population events: many neurons firing within a few ticks.

## Further reading

- Izhikevich, E. M. (2003), "Simple model of spiking neurons", *IEEE Trans. Neural Networks* 14(6), 1569.
- Izhikevich, E. M. (2007), *Dynamical Systems in Neuroscience*, MIT Press.
- Golomb, D. & Rinzel, J. (1993), "Dynamics of globally coupled inhibitory neurons with heterogeneity", *Phys. Rev. E* 48, 4810.
- Hodgkin, A. L. & Huxley, A. F. (1952), *J. Physiol.* 117, 500.
