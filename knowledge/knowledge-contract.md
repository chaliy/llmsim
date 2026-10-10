---
type: Playbook
title: Knowledge Maintenance Contract
description: Rules for maintaining LLMSim knowledge and OKF v0.2 conformance.
tags:
  - llmsim
  - knowledge
  - okf
  - process
---

# Knowledge Maintenance Contract

`knowledge/` is LLMSim's canonical [Open Knowledge Format (OKF) v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle and persistent project memory. It owns durable design intent, constraints,
feature contracts, rationale, and success bars. It replaces the former `specs/` folder.

## Maintenance rules

- Treat knowledge as part of the implementation, not as historical documentation.
- Read the relevant concepts before changing behavior. Update them in the same change
  when decisions, behavior, constraints, tests, or operations change.
- Record important decisions that are not recoverable from code. Prefer links to source
  and tests over duplicating volatile implementation details.
- Keep stable requirement identifiers such as `R1.2`; never renumber them.
- Keep concepts readable in one sitting. Split an oversized concept by audience or
  subsystem and link its parts from the appropriate domain index.
- Public user documentation remains in `docs/`.

## Knowledge boundaries

A concept owns the **why** and the **what**: intent, rationale, constraints, contracts,
success bars, and rejected options. Everything else has a better home:

| Content | Source of truth |
|---|---|
| struct fields, enum variants, model profile tables | Rust source in `src/` |
| request/response shapes of the simulated providers | the upstream provider API docs and `src/{openai,anthropic,openresponses}/` |
| commands, flags, and procedures | `justfile`, scripts, `.claude/skills/`, `.claude/commands/` |
| user-facing usage | `docs/` |

Link to those sources instead of copying them. Stale copies are worse than no copy.

## OKF conformance

The bundle targets OKF v0.2, declared by `okf_version: "0.2"` in the root
[`index.md`](index.md).

- Every Markdown file except reserved `index.md` and `log.md` files is a concept and
  starts with YAML frontmatter containing a non-empty `type`.
- Concepts also carry `title` and single-sentence `description` metadata. `tags` are
  recommended.
- Domain `index.md` files have no frontmatter. The root index may contain only
  `okf_version` frontmatter.
- Every index is a link list that enumerates the concepts and immediate subdirectories
  beside it, and nothing deeper.
- `log.md` records date-grouped updates under `## YYYY-MM-DD`, newest first.
- Prose belongs in concepts, not indexes.
- Concept links are relative and must resolve.
- There is no `specs/` folder at the repository root; design intent lives only here.

Run `just check-okf` after changing the bundle. CI runs both the repository checker and
the pinned upstream `okf-lint` implementation.

## See also

- [Routine Maintenance Specification](project/maintenance.md), which includes knowledge alignment
- [LLMSim Architecture](foundations/architecture.md)
