---
name: Feature request
about: Suggest an improvement for DiffLens
labels: enhancement
---

**Problem to solve**

What workflow or document situation is painful today?

**Proposed solution**

What would you like DiffLens to do? Keep it local-first: features must not require cloud services.

**Alternatives considered**

Any workarounds or other tools you've tried.

**Scope check**

- Does this touch network egress? (New downloads must be added to the pinned manifest + `core/src/download/` only.)
- Does it affect determinism of severity/diff results? (LLM-assisted features must not change diff or severity output.)
