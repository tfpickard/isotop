# View Armorial

The Roll of Arms: the process tree as the heraldry of houses. A house is a cgroup path, or a process group where cgroups are absent. Banners hang in reserved bays along the walls of a nave, above stalls and beneath a high vault. Each banner belongs to a process; its size follows memory, its movement follows CPU, and its charges follow thread count. Cadency marks describe descent. The arms themselves are generated decoration, not a claim that a service has a coat of arms.

![Armorial: banners of arms in a candlelit nave](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/armorial.webp)

![The README image of the same view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/armorial.webp)

## The system being modelled

A Unix process has a parent pid and a start time. These make a sampled tree, not a complete family history: a parent may have exited, a child may have been reparented, and short-lived siblings may never have been sampled. Linux cgroups additionally identify groups of processes by path. A house in this view means that grouping, not ownership or ancestry inferred from a name.

Heraldry supplies a compact visual language for identity and descent. A coat of arms has a field, possibly divided, ordinaries laid on it, and charges. A child can bear the parent's arms with a small mark for difference, a brisure. Repeating this operation makes descent visible without giving every descendant an unrelated design. A process with no sampled parent in the same house is a founder for that house's observed armorial path.

## History

Hereditary armorial bearings emerged in western Europe in the twelfth century. Seals, shields and surcoats made an individual recognisable where armour obscured the face. The association with identification in battle is well established; a single inventor or a single founding date is not.

Rolls of arms record bearings as pictures, descriptions, or both. The thirteenth-century Glover's Roll survives through later copies. The Caerlaverock Roll of 1300 describes participants in the siege in verse. A roll is an index of identities expressed through a deliberately restricted vocabulary, rather than a collection of arbitrary pictures.

The College of Arms was incorporated by Richard III in 1484. Its officers maintain genealogical and armorial records and grant arms under their authority. This program is not an officer of arms. Its grants are the result of hashing a string.

English cadency commonly assigns a label to the eldest son, a crescent to the second, a mullet to the third, a martlet to the fourth, an annulet to the fifth, a fleur-de-lis to the sixth, a rose to the seventh, a cross moline to the eighth and an octofoil to the ninth. Actual practice is less uniform than the tidy sequence implies. The view applies the sequence mechanically to processes, without attributing sex or social rank to them.

A funeral hatchment displays arms on a lozenge-shaped board associated with a death. Its black ground is a mourning convention, although historical backgrounds could encode which spouse survived. Here it means precisely state `Z`: the process has died but has not yet been reaped. The hall borrows its arrangement from the hanging banners above the stalls at St George's Chapel, Windsor; it does not reconstruct the chapel.

Heraldic writers also described abatements, including stains such as tenne and sanguine. That such abatements were almost never actually borne is a historical claim requiring more specific documentary support (unverified). The small stain on a positively niced process is a visual convention, not a moral judgement.

## Theory

**Tincture as a constraint.** The usual rule separates tinctures into two classes: metals, Or and Argent, and colours, Gules, Azure, Vert, Sable and Purpure. An object laid on another should belong to the opposite class. Represent every superposed layer by a vertex and every support relationship by an edge. Assigning classes is a 2-colouring problem: adjacent vertices must have different colours. A stack is a path, hence bipartite. A divided field makes the support vary spatially; counterchanging chooses the opposite pigment at each point rather than pretending that one solid tincture is legal over both halves.

The generated grammar does this locally. If the pigment below a charge is a metal, the charge uses the house's colour; if it is a colour, the charge uses the metal. An ordinary, a charge laid over an ordinary, and each nested difference obey the same operation. This makes the rule a property of the painter rather than a hope about which charges might intersect a division.

**A blazon as information.** A blazon is a description in a constrained language. Seven divisions, nine ordinary choices, five charge types, two metals and five colours yield at most $7\times9\times5\times2\times5=3150$ combinations before arrangements and counts, or about 11.6 bits. The house hash has many more bits than the visible design; two houses can therefore share arms. The text and the painting read the same structure. A solid layer is named by its tincture; a layer using both pigments is described as counterchanged. The description is not a cryptographic identifier.

**Cadency as a tree address.** A path from a founder to a descendant is a sequence of sibling ordinals. It is a positional address in a tree whose radix varies with the parent's child count. Nine distinct marks encode ordinals one to nine; the tenth and later repeat the mark cycle, and the ordinal remains in the text. At most two generations are painted. The full path remains in the blazon, because two visible marks cannot encode an arbitrarily deep tree. A crescent on a crescent has a different address from a lone crescent.

## Mapping

| Visual element | Measured source | Transform |
|---|---|---|
| House | `Process::cgroup`, else `Process::group` | One reserved bay for each admitted house; the full path seeds its arms |
| Cadency | `Process::parent`, `Process::started`, pid | Siblings sorted by start time, then pid; ordinals cycle through nine marks, with ordinals 10 and above retained in the blazon; at most two nested levels painted |
| Banner size | `Process::memory` | `s = min(1.1 + 0.055 sqrt(MiB), 2.8) / 2.8`; full cloth height `3.6 s`, width bounded by its reserved column |
| Cloth sway | `Process::cpu` | `c = cpu / (cpu + 100)`; amplitude `0.08 c` before perspective compression, frequency factor `0.6 + 1.4 c`; idle cloth is still |
| Charges | `Process::threads` | One for 0 or 1 thread, two for 2, three for 3 to 7; 8 or more becomes semy, a fixed strewn arrangement |
| Helm | `Process::kind` | Kernel: gold barred affronté; System: silver barred; Container: open steel visor; Session: closed steel profile |
| Funeral hatchment | `Process::state == 'Z'` | A lozenge board with a black mourning surround |
| Furled banner | `Process::state == 'T'` | Height reduced from `3.6 s` to `0.5 s` |
| Abatement stain | `Process::nice > 0` | A small tenne point; historical practice uncertain (unverified) |
| Unfurling and lowering | First observation and disappearance of an identity | One-second smoothstep easing; lowering gathers the cloth within its own reservation |
| Admission | Memory | `model::top_n`; at most 48 banners including those lowering, with the hidden live processes counted as collapsed |

**Decorative:** the particular arms granted from the path hash, the exact pigments and folds, the architecture, floor tiles, pillars, stall canopies, barrel vault, east window, poles, candles and warm glows. Helm shapes borrow a historical classification but encode only the measured process kind. A charge is not a report of a file, socket or service function.

## How isotop realises it

`src/armorial.rs` keeps a `#[derive(Default)]` `Armorial` module. `arms` maps the house-path hash to an `Arms` structure; `Division`, `Ordinary`, `Charge`, `Tincture` and `Brisure` are the grammar. `Arms::tincture_at` paints it; `Arms::blazon` describes it. `ancestry` computes sibling order from the complete sampled process list, before admission hides any processes. `tree_order` puts each founder before its descendants.

A banner is a `render::field::Field`, sampled at 121 nodes in each direction and displaced on a mesh capped at 13 vertices a side. Texture resolution and geometric resolution are independent. This preserves readable charges without emitting a triangle pair for every painted cell. Every owner node carries the bearer's index into the frame's identities. Closed-form sway, unfurling and lowering read scene time rather than accumulating frame steps. Texture caches for hidden live banners are discarded without releasing their seats.

The shared camera is orthographic. The module compresses the nave projectively: a physical point at distance $d$ down the aisle is scaled by $22/(22+d)$, including its height, and its along-aisle coordinate becomes $-d\,22/(22+d)$. The two walls, pavement and roof therefore converge at a common vanishing point without changing another view's projection. Column spacing gives the first eight columns more room before distant additions approach the window. This is an architectural drawing convention, not a measured distance.

Bays and banner seats are keyed by house path and `Identity`, respectively. Each column has two separate tiers, with space between their cloth envelopes. A survivor keeps its column and tier even if resource changes remove it from admission. A house reserves enough columns on admission and can reserve additional columns without moving the old ones. Exit animation shrinks the banner as it lowers so it stays in that reservation. Lowering banners reserve part of the 48-banner budget before live memory-ranked admission.

Selecting or touring a banner asks the module for wrapped blazon lines and its parent description. House titles use `render::label_name`; popup text follows the application's normal cleaning path. On macOS grouping falls back to `Process::group`, and memory is the physical footprint reported by that platform. No additional privileged or optional source is required. A missing parent ends the observed ancestry; the program does not invent a founder's parent.

## Reading the picture

Follow the chequered aisle toward the east window. The stalls and pillars define the two walls; poles project into the hall above them. A bay groups a house. Banners near the entrance appear larger because of the architectural perspective, so compare memory sizes within a nearby group rather than comparing the nearest and farthest cloth directly.

Movement indicates CPU activity; a motionless banner can still have many charges, because thread count does not imply CPU use. A small mark near the upper part of the arms is a difference. The second child's crescent is not the main charge of its house. A smaller mark painted within that mark encodes the next generation. Read the popup when the distinction is too small at terminal resolution, or when descent goes beyond two levels.

The black lozenge is a zombie, not a process that disappeared from the sample. The furled cloth is stopped. The stain means positive nice, not an error, bad behaviour, or an OOM ranking. The generated colours identify houses only decoratively: identical-looking arms are possible.

## Try it

```sh
./target/release/isotop --demo --view armorial --time 12 --width 1600 --renderer cpu --output /tmp/armorial-12.png
./target/release/isotop --demo --view armorial --time 20 --width 1600 --renderer cpu --output /tmp/armorial-20.png
./target/release/isotop --view armorial
```

Use the `v` picker to select Armorial. Hover or click a banner for its process; select it for the blazon and cadency. The ordinary global camera, focus and tour controls apply. The view adds no local keys.

## Where in the code

* `src/armorial.rs`: `Armorial`, `Bay`, `Banner`, `arms`, `ancestry`, `tree_order`, `Arms::tincture_at`, `Arms::tincture_word`, `Arms::blazon`, `Brisure`, `mark_centre`, `world`, `distance`, `solid` and `helm`.
* `src/render/field.rs`: `Field`, `Placement::Corners`, `Relief::Displaced`, and `Frame::field`.
* `src/model.rs`: `Process`, `Identity`, `Kind`, `bounded` and `top_n`.
* `src/views.rs`: the appended `armorial` registry row; `src/main.rs`: the module declaration.
* Tests in `src/armorial.rs`: rule of tincture over 10,000 seeds, sibling order, nested differences, hand-checked blazons, stable seats, deterministic grants, the lowering budget, frame-rate independence and unoccluded reservations. The shared harness checks registered-view rendering and picking.

## Variations and open questions

The sampled tree loses departed siblings. Remembering their ordinals could make cadency more historically persistent, but would change the meaning from current birth order among sampled siblings to an observed family history. Reparenting has the same distinction.

A full blazon could distinguish the conventional names of metal roundels, or use a larger grammar. The present limited vocabulary is easier to check against the painter. Increasing vocabulary would not make the grant a measurement.

Two nested marks are already small at full-hall scale. Zoom and the text carry more information than adding a third painted level would. A single house with many processes can reserve columns deep into the hall, where the perspective makes marks smaller. The architectural reference could also be made more exact, though doing so would add decoration rather than measured information.

## References

* A. C. Fox-Davies, *A Complete Guide to Heraldry* (1909).
* The College of Arms, institutional history and explanation of heraldry and grants of arms.
* The Caerlaverock Roll (1300), an early roll in verse.
* C. E. Shannon, "A mathematical theory of communication", *Bell System Technical Journal* 27 (1948).
* [Theory Stable Layout](Theory-Stable-Layout) and [Theory Text Safety](Theory-Text-Safety).
