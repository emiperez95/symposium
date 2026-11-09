# Project

## Introduction

@md/introduction.md

## Documentation

Design documentation is tracked in the mdbook:

@md/SUMMARY.md

The module structure and important flows chapters are particularly useful for understanding the codebase:

@md/design/module-structure.md

@md/design/important-flows.md

## Instructions

Agent MUST follow the following guidance:

* **Check common issues first**: Before starting a coding task, review `md/design/common-issues.md` for recurring bug patterns that may apply to your work.
* **Update design documentation**: Update the mdbook chapters in `md/design` as appropriate so that they are kept current. This will help both you and future agents to remember how things work.
* **Check that everything builds and don't forget tests**: After making changes, remember to check that the typescript + swift + Rust code builds and to run tests.
* **Keep requirement evidence honest**: When `cargo test --test habeas` fails because code quoted by `evidence/` changed, follow `md/design/requirements-workflow.md`: re-capture the input with `habeas add-input`, reread the requirement against the new code, and set `determination` to `true` only if it still holds. Never flip a determination or edit quoted text by hand to make the test pass.
* **Co-authorship**: Include "Co-authored-by" with your agent identifier (e.g., "Claude <claude@anthropic.com>") to indicate AI collaboration.
