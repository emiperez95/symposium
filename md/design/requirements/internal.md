# Internal requirements

Project-wide requirements for code that has no behavior of its own, following Habeas's guidance for helpers. Citing one never justifies end-user-observable behavior. Their evidence is checked by `cargo test` (see [Requirements and evidence](../requirements-workflow.md)).

:::{req} internal_detail
A function cited with this requirement MUST NOT introduce end-user-observable behavior beyond the requirements its callers cite.
:::

*Source:* Inferred from the code only (n/a: meta-requirement for accessors, wrappers and test helpers; nothing in the code base states it); needs maintainer review.
