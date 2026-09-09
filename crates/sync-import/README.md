# halogen-sync-import

Validation for importing durable mutations.

- Decode the complete batch before storing any operation.
- Bound operation count, identifier length and encoded payload size.
- Unknown or malformed operations remain with the caller for recovery.
