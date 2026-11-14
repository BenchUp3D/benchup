# Rounds 1–4 (12 November 2026)

Before the tools of [#42](https://github.com/BenchUp3D/benchup/issues/42). Each cell is the number of `benchup` commands the agent ran, with the commands that returned an error in brackets. Rounds 1–4 used the 12 original cases; the fixes between rounds are in the history of PR #41.

| # | Case | R1 | R2 | R3 | R4 | Result in R4 |
|---|---|---|---|---|---|---|
| 01 | Lengthen the desk | 43 (3) | 27 (0) | 19 (1) | ~61 (0) | ✅, a more ambitious solution; 36 commands spent finding the walls one by one |
| 02 | Five shelves to four | 21 (3) | 22 (0) | 16 (0) | 14 (0) | ✅, with `flip_joint` |
| 03 | Dining table | 22 (0) | 18 (0) | 15 (0) | 15 (0) | ✅ |
| 04 | Chest of drawers | 46 (3) | 32 (9) | 19 (0) | 14 (0) | ✅, one `--dry-run` |
| 05 | Back in a rabbet and plinth | 14 (0) | 27 (5) | 17 (0) | 18 (0) | ✅ |
| 06 | Solid oak 19 mm | 24 (3) | 25 (0) | 17 (0) | 17 (1) | ✅ |
| 07 | Bedside table with curved sides | ~261 (35) | 26 (1) | 24 (0) | 36 (4) | ✅, hit the core's precision limit (32 or 40 arc segments) |
| 08 | Leaning ladder shelf | ~91 (16) | 45 (1) | 32 (0) | 22 (0) | ✅, first try |
| 09 | Workshop dimensions | 21 (1) | 13 (0) | 12 (0) | 11 (0) | ✅, with `axis` |
| 10 | Modules, components and tags | ~142 (8) | 31 (5) | 27 (2) | 19 (~2) | ⚠️ partial on purpose (housed shelves cannot be one component) |
| 11 | Cut list and sheets | 8 (0) | 12 (0) | 8 (0) | 8 (0) | ✅ |
| 12 | A shelf that does not fit | 8 (0) | 7 (0) | 12 (0) | 10 (0) | ✅, with `passes_through` |
| | **Total** | **~701 (72)** | **285 (21)** | **218 (3)** | **~245 (~7)** | 11 ✅, 1 ⚠️ |

What remained after round 4 became [#42](https://github.com/BenchUp3D/benchup/issues/42).
