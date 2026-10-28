---
name: contributing
description: Write GitHub issues and PRs for the Symposium project and place them on the project board. Use when creating or reviewing a ticket (Task or Feature), breaking an initiative down into tickets, or opening a PR that implements one.
---

# Contributing tickets and PRs

The rules live in the book page `md/design/contributing-tickets.md`. Read it before writing a ticket; this skill only says where to look and why the pieces exist.

## Where things are

- Board statuses, issue types, and ticket layouts: `md/design/contributing-tickets.md`.
- Feature (initiative) issues: the issue form in `.github/ISSUE_TEMPLATE/initiative.yml`. Use the form rather than copying its fields, so the issue follows the form when it changes.
- PRs: `.github/PULL_REQUEST_TEMPLATE.md`. Answer its disclosure questions (see `md/design/governance.md`).
- Larger changes whose design is still contested: write an RFD first, using the `authoring-rfds` skill.

## Why the pieces exist

- **Requirements and examples** let someone review the ticket before code exists. Each requirement is one MUST/MUST NOT statement with a short name (`notice.once`); each example is a concrete situation, action and expected result citing a requirement. The vocabulary comes from Habeas Codicem; the repo does not use its evidence files or tooling, so do not add them.
- **Out of scope** and **Open questions** stop the implementer from guessing.
- **Code PRs** on a Feature, with each PR saying "Part of #N", tie the work back to the initiative.
- **The issue type** (Feature or Task) carries the size, so the description does not repeat it.

## Process

1. Decide: Task (design settled), RFD (design contested), or Feature (an initiative to break down).
2. Draft the description following the page's layout. Title in plain words, no `[tag]` prefixes or scenario numbers.
3. Get the description reviewed before building.
4. Open the PR from the template, linking the issue.

Never create, edit, or comment on issues or PRs on GitHub without the user's explicit approval: drafts go to the user first.

## Example: good vs. bad

Bad:
> **Size: ticket.** Improve the consent experience so users aren't annoyed.

Good:
> `notice.once`: Symposium MUST show the consent notice at most once per agent session.
>
> `notice.once`: two dependencies await consent; the user starts a session and sends three prompts; the notice appears once, at session start, naming both.
