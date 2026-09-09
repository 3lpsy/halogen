# Handlers

Podcast application actions over validated wire types and SeaORM models.

- Resource modules live directly under `src/` and own mutations and response assembly.
- Database import stages uploads, then merges entity phases in one transaction.
- Complex episode and playlist selection and raw snapshot SQL live in `queries`.
- HTTP extraction and request authorization live in `routes`, `extractors`
  and `guards`; actions retain ownership checks where they use referenced rows.
- No HTTP listener, static frontend or renderer dependency.
