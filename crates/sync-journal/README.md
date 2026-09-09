Durable operation import and command transaction staging.

- `decode_operations` validates an entire legacy batch before import writes begin.
- `BufferedStore` stages cache changes, operation enqueueing, and acknowledgements for one command.
- Reads see staged changes. `commit` writes the cache and journal together through `LocalStore::commit_changes`.
- Failed reads prevent commit; failed writes leave the underlying cache and previous delivery intent intact.

## Glossary

- Journal: pending or rejected operations retained for delivery and review.
- Acknowledgement: removal of a delivered or superseded operation.
- Command transaction: one cache mutation and its associated delivery operations.
